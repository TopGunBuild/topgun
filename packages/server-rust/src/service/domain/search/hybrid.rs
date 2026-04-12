//! Tri-hybrid search engine: types for SearchMethod, HybridSearchResult,
//! and HybridSearchError.
//!
//! `HybridSearchEngine` (the async orchestrator) is implemented separately,
//! after `SearchService::search_map` and `rrf::fuse` are available.

use std::collections::HashMap;

use crate::service::domain::embedding::EmbeddingError;

// ---------------------------------------------------------------------------
// SearchMethod
// ---------------------------------------------------------------------------

/// Which search method(s) to invoke.
///
/// serde derives are pre-positioned for the wire protocol follow-up spec that will add
/// HybridSearch/HybridSearchResp message variants. They are not used in this spec but
/// add negligible overhead and avoid a breaking rename when the follow-up lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMethod {
    /// Predicate-based exact match via IndexRegistry.
    Exact,
    /// BM25 full-text search via tantivy SearchService.
    FullText,
    /// Approximate nearest neighbor via VectorIndex.
    Semantic,
}

// ---------------------------------------------------------------------------
// HybridSearchResult
// ---------------------------------------------------------------------------

/// Result of a hybrid search query.
#[derive(Debug, Clone)]
pub struct HybridSearchResult {
    /// Record key.
    pub key: String,
    /// Fused RRF score.
    pub score: f64,
    /// Per-method scores for transparency/debugging.
    pub method_scores: HashMap<SearchMethod, f64>,
    /// Record value (populated when include_value is true).
    pub value: Option<rmpv::Value>,
}

// ---------------------------------------------------------------------------
// HybridSearchError
// ---------------------------------------------------------------------------

/// Error type for hybrid search operations.
#[derive(Debug, thiserror::Error)]
pub enum HybridSearchError {
    #[error("embedding failed: {0}")]
    Embedding(#[from] EmbeddingError),
    #[error("no embedding provider configured for semantic search")]
    NoEmbeddingProvider,
    #[error("map not found: {0}")]
    MapNotFound(String),
}
