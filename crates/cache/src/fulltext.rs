//! Tantivy full-text index wrapper.

use crate::error::{CacheError, CacheResult};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexWriter, ReloadPolicy};
use tantivy::directory::MmapDirectory;
use tracing::{info, debug, warn, instrument};

/// Tantivy-backed full-text search index.
pub struct FulltextIndex {
    index: Index,
    _schema: Schema,
    // Field handles for quick access
    f_url_hash: Field,
    f_url: Field,
    f_title: Field,
    f_body: Field,
    f_snippet: Field,
    f_tags: Field,
    f_source: Field,
}

/// A raw search hit from Tantivy (before enrichment from SQLite).
pub struct FulltextHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

impl FulltextIndex {
    pub fn open(path: &Path) -> CacheResult<Self> {
        std::fs::create_dir_all(path)
            .map_err(|e| CacheError::IndexError(format!("Failed to create index dir: {e}")))?;

        let mut schema_builder = Schema::builder();

        let f_url_hash = schema_builder.add_text_field("url_hash", STRING | STORED);
        let f_url = schema_builder.add_text_field("url", TEXT | STORED);
        let f_title = schema_builder.add_text_field("title", TEXT | STORED);
        let f_body = schema_builder.add_text_field("body", TEXT);
        let f_snippet = schema_builder.add_text_field("snippet", TEXT | STORED);
        let f_tags = schema_builder.add_text_field("tags", TEXT);
        let f_source = schema_builder.add_text_field("source", STRING | STORED);

        let schema = schema_builder.build();

        let mmap_dir = MmapDirectory::open(path)
            .map_err(|e| CacheError::IndexError(format!("MmapDirectory open failed: {e}")))?;

        let index = if Index::exists(&mmap_dir)
            .map_err(|e| CacheError::IndexError(format!("Index check failed: {e}")))?
        {
            Index::open_in_dir(path)
                .map_err(|e| CacheError::IndexError(format!("Failed to open index: {e}")))?
        } else {
            Index::create_in_dir(path, schema.clone())
                .map_err(|e| CacheError::IndexError(format!("Failed to create index: {e}")))?
        };

        info!(path = %path.display(), "Tantivy fulltext index opened");

        Ok(Self {
            index,
            _schema: schema,
            f_url_hash,
            f_url,
            f_title,
            f_body,
            f_snippet,
            f_tags,
            f_source,
        })
    }

    fn writer(&self) -> CacheResult<IndexWriter> {
        self.index
            .writer(15_000_000) // 15MB heap
            .map_err(|e| CacheError::IndexError(format!("Failed to create writer: {e}")))
    }

    #[instrument(skip(self, plain_text), fields(url = %url))]
    pub fn index_page(
        &self,
        url_hash: &str,
        url: &str,
        title: &str,
        plain_text: &str,
        snippet: &str,
        tags: &[String],
    ) -> CacheResult<()> {
        debug!(url = %url, text_len = plain_text.len(), "Indexing page in Tantivy");

        let mut writer = self.writer()?;

        // Delete existing document with same url_hash
        let term = tantivy::Term::from_field_text(self.f_url_hash, url_hash);
        writer.delete_term(term);

        let tags_str = tags.join(" ");
        writer.add_document(doc!(
            self.f_url_hash => url_hash,
            self.f_url => url,
            self.f_title => title,
            self.f_body => plain_text,
            self.f_snippet => snippet,
            self.f_tags => tags_str.as_str(),
            self.f_source => "page",
        )).map_err(|e| CacheError::IndexError(format!("Add doc failed: {e}")))?;

        writer.commit()
            .map_err(|e| CacheError::IndexError(format!("Commit failed: {e}")))?;

        Ok(())
    }

    pub fn index_search_result(
        &self,
        query_hash: &str,
        query: &str,
        title: &str,
        snippet: &str,
        url: &str,
    ) -> CacheResult<()> {
        debug!(query = %query, url = %url, "Indexing search result in Tantivy");

        let mut writer = self.writer()?;

        writer.add_document(doc!(
            self.f_url_hash => query_hash,
            self.f_url => url,
            self.f_title => title,
            self.f_body => query,
            self.f_snippet => snippet,
            self.f_source => "search",
        )).map_err(|e| CacheError::IndexError(format!("Add doc failed: {e}")))?;

        writer.commit()
            .map_err(|e| CacheError::IndexError(format!("Commit failed: {e}")))?;

        Ok(())
    }

    #[instrument(skip(self), fields(query = %query))]
    pub fn search(&self, query: &str, max_results: usize) -> CacheResult<Vec<FulltextHit>> {
        debug!(query = %query, max = max_results, "Searching Tantivy index");

        let reader = self.index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| CacheError::IndexError(format!("Reader failed: {e}")))?;

        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.f_title, self.f_body, self.f_snippet, self.f_url],
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
            .map_err(|e| CacheError::IndexError(format!("Search failed: {e}")))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| CacheError::IndexError(format!("Doc fetch failed: {e}")))?;

            let url = doc
                .get_first(self.f_url)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = doc
                .get_first(self.f_title)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = doc
                .get_first(self.f_snippet)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            results.push(FulltextHit {
                url,
                title,
                snippet,
                score,
            });
        }

        debug!(hits = results.len(), "Tantivy search complete");
        Ok(results)
    }

    /// Delete all documents for a given url_hash.
    pub fn delete(&self, url_hash: &str) -> CacheResult<()> {
        let mut writer = self.writer()?;
        let term = tantivy::Term::from_field_text(self.f_url_hash, url_hash);
        writer.delete_term(term);
        writer.commit()
            .map_err(|e| CacheError::IndexError(format!("Commit failed: {e}")))?;
        Ok(())
    }
}
