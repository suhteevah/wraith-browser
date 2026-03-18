//! Local Tantivy index for browsing history and extracted content.
//! Enables RAG-style retrieval: "search my browsing history for..."

use crate::{SearchResult, error::SearchError};
use tracing::instrument;

#[instrument(skip_all, fields(query = %query))]
pub fn search_local(query: &str, _max_results: usize) -> Result<Vec<SearchResult>, SearchError> {
    tracing::info!(query = %query, "Searching local index");
    // TODO: Tantivy query execution
    Ok(vec![])
}

pub fn index_page(url: &str, title: &str, _content: &str) -> Result<(), SearchError> {
    tracing::debug!(url = %url, title = %title, "Indexing page to local store");
    // TODO: Tantivy document insertion
    Ok(())
}
