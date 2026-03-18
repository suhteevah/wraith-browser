//! Local Tantivy index for browsing history and extracted content.
//! Enables RAG-style retrieval: "search my browsing history for..."
//!
//! This module provides a standalone Tantivy index at `~/.openclaw/local_index/`.
//! It is also the entry point for the KnowledgeStore's fulltext search — when a
//! KnowledgeStore exists, `search_local` queries both the standalone index AND
//! the cache's Tantivy index for maximum recall.

use crate::{SearchResult, SearchSource, error::SearchError};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexWriter, ReloadPolicy};
use tantivy::directory::MmapDirectory;
use tracing::{info, debug, warn, instrument};

/// Schema field handles for the local index.
struct LocalIndex {
    index: Index,
    f_url: Field,
    f_title: Field,
    f_body: Field,
    f_snippet: Field,
}

/// Get the default index directory.
fn default_index_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".openclaw")
        .join("local_index")
}

/// Get the KnowledgeStore's Tantivy index directory.
fn knowledge_store_index_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".openclaw")
        .join("knowledge")
        .join("index")
}

/// Open (or create) a Tantivy index at the given path.
fn open_local_index(path: &Path) -> Result<LocalIndex, SearchError> {
    std::fs::create_dir_all(path)
        .map_err(|e| SearchError::IndexError(format!("Failed to create index dir: {e}")))?;

    let mut schema_builder = Schema::builder();
    let f_url = schema_builder.add_text_field("url", TEXT | STORED);
    let f_title = schema_builder.add_text_field("title", TEXT | STORED);
    let f_body = schema_builder.add_text_field("body", TEXT);
    let f_snippet = schema_builder.add_text_field("snippet", TEXT | STORED);
    let schema = schema_builder.build();

    let mmap_dir = MmapDirectory::open(path)
        .map_err(|e| SearchError::IndexError(format!("MmapDirectory open failed: {e}")))?;

    let index = if Index::exists(&mmap_dir)
        .map_err(|e| SearchError::IndexError(format!("Index check failed: {e}")))?
    {
        Index::open_in_dir(path)
            .map_err(|e| SearchError::IndexError(format!("Failed to open index: {e}")))?
    } else {
        Index::create_in_dir(path, schema.clone())
            .map_err(|e| SearchError::IndexError(format!("Failed to create index: {e}")))?
    };

    Ok(LocalIndex { index, f_url, f_title, f_body, f_snippet })
}

/// Search a single Tantivy index, returning results.
fn search_index(
    local: &LocalIndex,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let reader = local.index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
        .map_err(|e| SearchError::IndexError(format!("Reader creation failed: {e}")))?;

    let searcher = reader.searcher();

    let query_parser = QueryParser::for_index(
        &local.index,
        vec![local.f_title, local.f_body, local.f_snippet, local.f_url],
    );

    let parsed = match query_parser.parse_query(query) {
        Ok(q) => q,
        Err(e) => {
            warn!(query = %query, error = %e, "Query parse failed, returning empty");
            return Ok(vec![]);
        }
    };

    let top_docs = searcher
        .search(&parsed, &TopDocs::with_limit(max_results))
        .map_err(|e| SearchError::IndexError(format!("Search failed: {e}")))?;

    let mut results = Vec::with_capacity(top_docs.len());
    for (score, doc_address) in top_docs {
        let doc: TantivyDocument = searcher
            .doc(doc_address)
            .map_err(|e| SearchError::IndexError(format!("Doc fetch failed: {e}")))?;

        let url = doc.get_first(local.f_url)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = doc.get_first(local.f_title)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let snippet = doc.get_first(local.f_snippet)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        results.push(SearchResult {
            title,
            url,
            snippet,
            source: SearchSource::LocalIndex,
            relevance_score: score,
        });
    }

    Ok(results)
}

/// Search the local browsing history index AND the KnowledgeStore's fulltext index.
#[instrument(skip_all, fields(query = %query))]
pub fn search_local(query: &str, max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    info!(query = %query, "Searching local indexes");

    let mut all_results = Vec::new();

    // 1. Search the standalone local index
    let local_path = default_index_path();
    if local_path.exists() {
        match open_local_index(&local_path) {
            Ok(local) => {
                match search_index(&local, query, max_results) {
                    Ok(results) => {
                        debug!(hits = results.len(), "Standalone local index results");
                        all_results.extend(results);
                    }
                    Err(e) => warn!(error = %e, "Standalone local index search failed"),
                }
            }
            Err(e) => warn!(error = %e, "Failed to open standalone local index"),
        }
    }

    // 2. Search the KnowledgeStore's Tantivy index (different schema)
    let ks_path = knowledge_store_index_path();
    if ks_path.exists() {
        match search_knowledge_store_index(&ks_path, query, max_results) {
            Ok(results) => {
                debug!(hits = results.len(), "KnowledgeStore index results");
                all_results.extend(results);
            }
            Err(e) => warn!(error = %e, "KnowledgeStore index search failed"),
        }
    }

    // Merge, deduplicate by URL, sort by score
    all_results.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_results.dedup_by(|a, b| a.url == b.url);
    all_results.truncate(max_results);

    debug!(total_hits = all_results.len(), "Local search complete");
    Ok(all_results)
}

/// Search the KnowledgeStore's Tantivy index.
/// The KS index has a different schema (url_hash, url, title, body, snippet, tags, source).
fn search_knowledge_store_index(
    path: &Path,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let mmap_dir = MmapDirectory::open(path)
        .map_err(|e| SearchError::IndexError(format!("KS MmapDirectory open failed: {e}")))?;

    if !Index::exists(&mmap_dir)
        .map_err(|e| SearchError::IndexError(format!("KS index check failed: {e}")))?
    {
        return Ok(vec![]);
    }

    let index = Index::open_in_dir(path)
        .map_err(|e| SearchError::IndexError(format!("KS index open failed: {e}")))?;

    let schema = index.schema();

    // The KS index has fields: url_hash, url, title, body, snippet, tags, source
    let f_url = match schema.get_field("url") {
        Ok(f) => f,
        Err(_) => return Ok(vec![]),
    };
    let f_title = match schema.get_field("title") {
        Ok(f) => f,
        Err(_) => return Ok(vec![]),
    };
    let f_body = match schema.get_field("body") {
        Ok(f) => f,
        Err(_) => return Ok(vec![]),
    };
    let f_snippet = match schema.get_field("snippet") {
        Ok(f) => f,
        Err(_) => return Ok(vec![]),
    };

    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::OnCommitWithDelay)
        .try_into()
        .map_err(|e| SearchError::IndexError(format!("KS reader failed: {e}")))?;

    let searcher = reader.searcher();

    let query_parser = QueryParser::for_index(
        &index,
        vec![f_title, f_body, f_snippet, f_url],
    );

    let parsed = match query_parser.parse_query(query) {
        Ok(q) => q,
        Err(e) => {
            warn!(query = %query, error = %e, "KS query parse failed");
            return Ok(vec![]);
        }
    };

    let top_docs = searcher
        .search(&parsed, &TopDocs::with_limit(max_results))
        .map_err(|e| SearchError::IndexError(format!("KS search failed: {e}")))?;

    let mut results = Vec::with_capacity(top_docs.len());
    for (score, doc_address) in top_docs {
        let doc: TantivyDocument = searcher
            .doc(doc_address)
            .map_err(|e| SearchError::IndexError(format!("KS doc fetch failed: {e}")))?;

        let url = doc.get_first(f_url)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = doc.get_first(f_title)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let snippet = doc.get_first(f_snippet)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        results.push(SearchResult {
            title,
            url,
            snippet,
            source: SearchSource::LocalIndex,
            relevance_score: score,
        });
    }

    Ok(results)
}

/// Index a page into the local browsing history index.
#[instrument(skip(content), fields(url = %url, title = %title))]
pub fn index_page(url: &str, title: &str, content: &str) -> Result<(), SearchError> {
    debug!(url = %url, title = %title, content_len = content.len(), "Indexing page to local store");

    let path = default_index_path();
    let local = open_local_index(&path)?;

    let mut writer: IndexWriter = local.index
        .writer(15_000_000) // 15MB heap
        .map_err(|e| SearchError::IndexError(format!("Writer creation failed: {e}")))?;

    // Generate a snippet from the first ~200 chars of content
    let snippet: String = content.chars().take(200).collect::<String>()
        .replace('\n', " ")
        .trim()
        .to_string();

    writer.add_document(doc!(
        local.f_url => url,
        local.f_title => title,
        local.f_body => content,
        local.f_snippet => snippet.as_str(),
    )).map_err(|e| SearchError::IndexError(format!("Add doc failed: {e}")))?;

    writer.commit()
        .map_err(|e| SearchError::IndexError(format!("Commit failed: {e}")))?;

    info!(url = %url, "Page indexed in local store");
    Ok(())
}
