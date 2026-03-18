//! # Semantic Embeddings for RAG Knowledge Base
//!
//! Computes vector embeddings for cached pages to enable semantic similarity
//! search — "find pages similar to X" without keyword matching. This is the
//! foundation for the Browser-Native RAG Knowledge Base (Architecture Pillar 5).
//!
//! ## Backend Architecture
//!
//! ```text
//! Text ──► EmbeddingBackend ──► Vec<f32> (embedding vector)
//!          │
//!          ├── MockEmbeddingBackend (testing — returns deterministic vectors)
//!          ├── FastEmbedBackend (ONNX, BGE/E5 quantized — requires fastembed)
//!          └── ApiEmbeddingBackend (OpenAI/Anthropic/Cohere embedding APIs)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn, instrument};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A computed embedding vector for a piece of content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embedding {
    /// The URL or ID of the content this embedding represents
    pub source_id: String,
    /// The embedding vector
    pub vector: Vec<f32>,
    /// Model used to compute this embedding
    pub model: String,
    /// Dimensionality of the vector
    pub dimensions: usize,
}

/// Result of a similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarityResult {
    /// Source ID of the matched content
    pub source_id: String,
    /// Cosine similarity score (0.0 - 1.0)
    pub similarity: f64,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// Trait for embedding model backends.
pub trait EmbeddingBackend: Send + Sync {
    /// Compute an embedding vector for a text.
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;

    /// Compute embeddings for multiple texts (batch).
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Get the model name.
    fn model_name(&self) -> &str;

    /// Get the embedding dimensionality.
    fn dimensions(&self) -> usize;

    /// Check if the backend is available.
    fn is_available(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Embedding store
// ---------------------------------------------------------------------------

/// In-memory store for embeddings with similarity search.
pub struct EmbeddingStore {
    embeddings: Vec<Embedding>,
    index: HashMap<String, usize>, // source_id → index
}

impl Default for EmbeddingStore {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddingStore {
    /// Create an empty embedding store.
    pub fn new() -> Self {
        Self {
            embeddings: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Add or update an embedding.
    #[instrument(skip(self, embedding), fields(source_id = %embedding.source_id, dims = embedding.dimensions))]
    pub fn upsert(&mut self, embedding: Embedding) {
        let id = embedding.source_id.clone();
        if let Some(&idx) = self.index.get(&id) {
            debug!(source_id = %id, "Updating existing embedding");
            self.embeddings[idx] = embedding;
        } else {
            let idx = self.embeddings.len();
            debug!(source_id = %id, idx, "Adding new embedding");
            self.index.insert(id, idx);
            self.embeddings.push(embedding);
        }
    }

    /// Get an embedding by source ID.
    pub fn get(&self, source_id: &str) -> Option<&Embedding> {
        self.index.get(source_id).map(|&idx| &self.embeddings[idx])
    }

    /// Number of stored embeddings.
    pub fn len(&self) -> usize {
        self.embeddings.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.embeddings.is_empty()
    }

    /// Find the most similar embeddings to a query vector.
    #[instrument(skip(self, query_vector), fields(top_k))]
    pub fn search(&self, query_vector: &[f32], top_k: usize) -> Vec<SimilarityResult> {
        let mut results: Vec<SimilarityResult> = self
            .embeddings
            .iter()
            .map(|emb| SimilarityResult {
                source_id: emb.source_id.clone(),
                similarity: cosine_similarity(query_vector, &emb.vector),
            })
            .collect();

        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(top_k);
        debug!(results = results.len(), "Similarity search complete");
        results
    }

    /// Find pages similar to a given source ID.
    #[instrument(skip(self), fields(source_id = %source_id, top_k))]
    pub fn find_similar(&self, source_id: &str, top_k: usize) -> Vec<SimilarityResult> {
        if let Some(emb) = self.get(source_id) {
            let vector = emb.vector.clone();
            self.search(&vector, top_k + 1)
                .into_iter()
                .filter(|r| r.source_id != source_id)
                .take(top_k)
                .collect()
        } else {
            warn!(source_id = %source_id, "Source not found for similarity search");
            vec![]
        }
    }

    /// Remove an embedding.
    pub fn remove(&mut self, source_id: &str) -> bool {
        if let Some(&idx) = self.index.get(source_id) {
            self.embeddings.swap_remove(idx);
            self.index.remove(source_id);
            // Fix index for swapped element
            if idx < self.embeddings.len() {
                let swapped_id = self.embeddings[idx].source_id.clone();
                self.index.insert(swapped_id, idx);
            }
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Math utilities
// ---------------------------------------------------------------------------

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let norm_a: f64 = a.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Euclidean distance between two vectors.
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return f64::MAX;
    }

    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = (*x as f64) - (*y as f64);
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

/// Normalize a vector to unit length.
pub fn normalize_vector(v: &mut [f32]) {
    let norm: f64 = v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x = (*x as f64 / norm) as f32;
        }
    }
}

// ---------------------------------------------------------------------------
// Mock backend
// ---------------------------------------------------------------------------

/// Mock embedding backend for testing.
/// Generates deterministic embeddings based on text hash.
pub struct MockEmbeddingBackend {
    dims: usize,
}

impl Default for MockEmbeddingBackend {
    fn default() -> Self {
        Self::new(128)
    }
}

impl MockEmbeddingBackend {
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }
}

impl EmbeddingBackend for MockEmbeddingBackend {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        // Generate deterministic pseudo-embedding from text hash
        let hash = blake3::hash(text.as_bytes());
        let bytes = hash.as_bytes();
        let mut vector = Vec::with_capacity(self.dims);

        for i in 0..self.dims {
            let byte_idx = i % 32;
            let val = (bytes[byte_idx] as f32 / 255.0) * 2.0 - 1.0; // normalize to [-1, 1]
            vector.push(val);
        }

        // Normalize to unit vector
        normalize_vector(&mut vector);
        Ok(vector)
    }

    fn model_name(&self) -> &str {
        "mock-embedding"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// FastEmbed backend stub (requires fastembed crate).
pub struct FastEmbedBackend {
    _model_name: String,
}

impl FastEmbedBackend {
    pub fn new(model_name: &str) -> Self {
        Self {
            _model_name: model_name.to_string(),
        }
    }
}

impl EmbeddingBackend for FastEmbedBackend {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
        Err("FastEmbed backend not available — compile with --features embeddings".to_string())
    }

    fn model_name(&self) -> &str {
        &self._model_name
    }

    fn dimensions(&self) -> usize {
        384 // BGE-small default
    }

    fn is_available(&self) -> bool {
        false
    }
}

/// API-based embedding backend (OpenAI, Cohere, etc.).
pub struct ApiEmbeddingBackend {
    _endpoint: String,
    _api_key: String,
    _model: String,
    dims: usize,
}

impl ApiEmbeddingBackend {
    pub fn new(endpoint: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            _endpoint: endpoint.to_string(),
            _api_key: api_key.to_string(),
            _model: model.to_string(),
            dims,
        }
    }
}

impl EmbeddingBackend for ApiEmbeddingBackend {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
        Err("API embedding backend requires async runtime — use embed_async".to_string())
    }

    fn model_name(&self) -> &str {
        &self._model
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn is_available(&self) -> bool {
        !self._api_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 0.001);
    }

    #[test]
    fn euclidean_distance_same() {
        let a = vec![1.0, 2.0, 3.0];
        assert!(euclidean_distance(&a, &a) < 0.001);
    }

    #[test]
    fn normalize_vector_unit() {
        let mut v = vec![3.0, 4.0];
        normalize_vector(&mut v);
        let norm: f64 = v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn mock_backend_deterministic() {
        let backend = MockEmbeddingBackend::new(128);
        let v1 = backend.embed("hello world").unwrap();
        let v2 = backend.embed("hello world").unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn mock_backend_different_texts_differ() {
        let backend = MockEmbeddingBackend::new(128);
        let v1 = backend.embed("hello").unwrap();
        let v2 = backend.embed("goodbye").unwrap();
        assert_ne!(v1, v2);
    }

    #[test]
    fn mock_backend_dimensions() {
        let backend = MockEmbeddingBackend::new(256);
        assert_eq!(backend.dimensions(), 256);
        let v = backend.embed("test").unwrap();
        assert_eq!(v.len(), 256);
    }

    #[test]
    fn embedding_store_upsert_and_get() {
        let mut store = EmbeddingStore::new();
        store.upsert(Embedding {
            source_id: "page1".to_string(),
            vector: vec![1.0, 0.0, 0.0],
            model: "test".to_string(),
            dimensions: 3,
        });
        assert_eq!(store.len(), 1);
        assert!(store.get("page1").is_some());
        assert!(store.get("page2").is_none());
    }

    #[test]
    fn embedding_store_search() {
        let mut store = EmbeddingStore::new();
        store.upsert(Embedding {
            source_id: "a".to_string(),
            vector: vec![1.0, 0.0, 0.0],
            model: "test".to_string(),
            dimensions: 3,
        });
        store.upsert(Embedding {
            source_id: "b".to_string(),
            vector: vec![0.9, 0.1, 0.0],
            model: "test".to_string(),
            dimensions: 3,
        });
        store.upsert(Embedding {
            source_id: "c".to_string(),
            vector: vec![0.0, 0.0, 1.0],
            model: "test".to_string(),
            dimensions: 3,
        });

        let results = store.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source_id, "a"); // most similar
        assert_eq!(results[1].source_id, "b"); // second most
    }

    #[test]
    fn embedding_store_find_similar() {
        let mut store = EmbeddingStore::new();
        store.upsert(Embedding {
            source_id: "a".to_string(),
            vector: vec![1.0, 0.0],
            model: "test".to_string(),
            dimensions: 2,
        });
        store.upsert(Embedding {
            source_id: "b".to_string(),
            vector: vec![0.95, 0.05],
            model: "test".to_string(),
            dimensions: 2,
        });

        let similar = store.find_similar("a", 1);
        assert_eq!(similar.len(), 1);
        assert_eq!(similar[0].source_id, "b");
    }

    #[test]
    fn embedding_store_remove() {
        let mut store = EmbeddingStore::new();
        store.upsert(Embedding {
            source_id: "x".to_string(),
            vector: vec![1.0],
            model: "test".to_string(),
            dimensions: 1,
        });
        assert!(store.remove("x"));
        assert_eq!(store.len(), 0);
        assert!(!store.remove("x"));
    }

    #[test]
    fn fastembed_not_available() {
        let backend = FastEmbedBackend::new("bge-small-en-v1.5");
        assert!(!backend.is_available());
        assert!(backend.embed("test").is_err());
    }
}
