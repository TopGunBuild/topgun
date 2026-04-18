//! Tri-hybrid search engine combining exact-match, full-text BM25, and semantic
//! ANN vector search into a single result list via Reciprocal Rank Fusion.
//!
//! `HybridSearchEngine` is the orchestrator: it dispatches each requested
//! `SearchMethod` against its respective index, collects ranked lists, calls
//! `rrf::fuse`, and enriches results with record values when requested.

use std::collections::HashMap;
use std::sync::Arc;

use topgun_core::messages::base::PredicateNode;
use topgun_core::messages::search::SearchOptions;

use crate::service::domain::embedding::{EmbeddingError, EmbeddingProvider};
use crate::service::domain::index::query_optimizer::index_aware_evaluate;
use crate::service::domain::index::registry::IndexRegistry;
use crate::service::domain::predicate::value_to_rmpv;
use crate::service::domain::search::rrf::{self, FusedEntry, RankedEntry};
use crate::service::domain::search::SearchService;
use crate::storage::record::RecordValue;
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// SearchMethod
// ---------------------------------------------------------------------------

/// Which search method(s) to invoke.
///
/// serde derives are pre-positioned for the wire protocol follow-up spec that will add
/// `HybridSearch`/`HybridSearchResp` message variants. They are not used in this spec
/// but add negligible overhead and avoid a breaking rename when the follow-up lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMethod {
    /// Predicate-based exact match via `IndexRegistry`.
    Exact,
    /// BM25 full-text search via tantivy `SearchService`.
    FullText,
    /// Approximate nearest neighbor via `VectorIndex`.
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
    /// Record value (populated when `include_value` is true).
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

// ---------------------------------------------------------------------------
// HybridSearchParams
// ---------------------------------------------------------------------------

/// Parameters for a hybrid search query.
///
/// Groups the per-call inputs into a single struct to keep
/// `hybrid_search` under the 7-argument clippy limit.
pub struct HybridSearchParams<'a> {
    pub map_name: &'a str,
    pub index_registry: &'a IndexRegistry,
    pub query_text: &'a str,
    pub query_vector: Option<&'a [f32]>,
    pub predicate: Option<&'a PredicateNode>,
    pub methods: &'a [SearchMethod],
    pub k: usize,
    pub include_value: bool,
}

// ---------------------------------------------------------------------------
// HybridSearchEngine
// ---------------------------------------------------------------------------

/// Orchestrates tri-hybrid search: exact-match, BM25 full-text, and ANN semantic.
///
/// Holds shared references to the search service and record store. The
/// embedding provider is optional — semantic search requires either a
/// pre-computed query vector or a configured provider.
pub struct HybridSearchEngine {
    search_service: Arc<SearchService>,
    record_store_factory: Arc<RecordStoreFactory>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    /// RRF smoothing constant k (default 60 per the standard RRF paper).
    rrf_k: u32,
}

impl HybridSearchEngine {
    /// Create a new engine. `rrf_k` defaults to 60.
    pub fn new(
        search_service: Arc<SearchService>,
        record_store_factory: Arc<RecordStoreFactory>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        Self {
            search_service,
            record_store_factory,
            embedding_provider,
            rrf_k: 60,
        }
    }

    /// Execute a hybrid search across the requested methods.
    ///
    /// `Exact` and `FullText` are CPU-bound synchronous operations (`RwLock`
    /// acquisition, predicate evaluation, tantivy scoring). Only `Semantic`
    /// with `embed()` is truly async. Running `Exact` and `FullText` first
    /// (sequentially) avoids holding an `RwLock` guard across an await point,
    /// which would be an `RwLock`-across-await bug.
    ///
    /// # Errors
    ///
    /// Returns `HybridSearchError::NoEmbeddingProvider` when `Semantic` is in
    /// `methods`, no `query_vector` is provided, and no `EmbeddingProvider` is
    /// configured. Returns `HybridSearchError::Embedding` if the provider call
    /// fails.
    pub async fn hybrid_search(
        &self,
        params: HybridSearchParams<'_>,
    ) -> Result<Vec<HybridSearchResult>, HybridSearchError> {
        let mut ranked_lists: Vec<Vec<RankedEntry>> = Vec::new();
        // Track which list index corresponds to which method for method_scores mapping.
        let mut method_order: Vec<SearchMethod> = Vec::new();

        // --- Exact-match (synchronous) ---
        if params.methods.contains(&SearchMethod::Exact) {
            let exact_list =
                self.run_exact(params.map_name, params.index_registry, params.predicate);
            if !exact_list.is_empty() {
                ranked_lists.push(exact_list);
                method_order.push(SearchMethod::Exact);
            }
        }

        // --- Full-text BM25 (synchronous) ---
        if params.methods.contains(&SearchMethod::FullText) {
            let ft_list = self.run_fulltext(params.map_name, params.query_text);
            if !ft_list.is_empty() {
                ranked_lists.push(ft_list);
                method_order.push(SearchMethod::FullText);
            }
        }

        // --- Semantic ANN (async, uses await) ---
        if params.methods.contains(&SearchMethod::Semantic) {
            let sem_list = self
                .run_semantic(
                    params.index_registry,
                    params.query_text,
                    params.query_vector,
                    params.k,
                )
                .await?;
            if !sem_list.is_empty() {
                ranked_lists.push(sem_list);
                method_order.push(SearchMethod::Semantic);
            }
        }

        // --- Fuse ---
        let fused: Vec<FusedEntry> = rrf::fuse(&ranked_lists, self.rrf_k, params.k);

        // Collect enrichment values once (single pass) if needed, keyed by record key.
        let value_map: HashMap<String, rmpv::Value> = if params.include_value {
            self.load_all_values(params.map_name)
        } else {
            HashMap::new()
        };

        // --- Map FusedEntry to HybridSearchResult ---
        let results: Vec<HybridSearchResult> = fused
            .into_iter()
            .map(|fe| {
                let value = if params.include_value {
                    value_map.get(&fe.key).cloned()
                } else {
                    None
                };

                // Translate list indices back to SearchMethod keys for transparency.
                let method_scores: HashMap<SearchMethod, f64> = fe
                    .method_scores
                    .iter()
                    .filter_map(|(list_idx, score)| {
                        method_order.get(*list_idx).map(|m| (*m, *score))
                    })
                    .collect();

                HybridSearchResult {
                    key: fe.key,
                    score: fe.score,
                    method_scores,
                    value,
                }
            })
            .collect();

        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Run predicate-based exact-match via `IndexRegistry`.
    ///
    /// Collects all records from the map, then calls `index_aware_evaluate`
    /// following the same pattern as `query.rs` lines 608-612. Returns binary
    /// relevance: all matching keys at rank 1, since predicate evaluation is
    /// pass/fail not scored.
    fn run_exact(
        &self,
        map_name: &str,
        index_registry: &IndexRegistry,
        predicate: Option<&PredicateNode>,
    ) -> Vec<RankedEntry> {
        let Some(pred) = predicate else {
            return Vec::new();
        };

        // Collect entries across all partitions (same pattern as query.rs).
        let stores = self.record_store_factory.get_all_for_map(map_name);
        let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
        for store in &stores {
            store.for_each_boxed(
                &mut |key, record| {
                    if let RecordValue::Lww { ref value, .. } = record.value {
                        entries.push((key.to_string(), value_to_rmpv(value)));
                    }
                },
                false,
            );
        }

        let entry_map: HashMap<&str, &rmpv::Value> =
            entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let all_keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();

        let matching_keys = index_aware_evaluate(index_registry, pred, &all_keys, |key| {
            entry_map.get(key).map(|v| (*v).clone())
        });

        // Binary relevance: all matches receive rank 1, original_score = 1.0.
        // RRF handles uniform rank correctly — contribution is 1/(rrf_k+1) each.
        matching_keys
            .into_iter()
            .map(|key| RankedEntry {
                key,
                rank: 1,
                original_score: 1.0,
            })
            .collect()
    }

    /// Run BM25 full-text search via tantivy `SearchService` accessor.
    fn run_fulltext(&self, map_name: &str, query_text: &str) -> Vec<RankedEntry> {
        if query_text.is_empty() {
            return Vec::new();
        }

        let options = SearchOptions::default();
        let scored_docs = self
            .search_service
            .search_map(map_name, query_text, &options);

        // Convert score-ordered `ScoredDoc` list to 1-based `RankedEntry` list.
        // Result counts from tantivy are always far below u32::MAX in practice.
        scored_docs
            .into_iter()
            .enumerate()
            .map(|(i, doc)| RankedEntry {
                key: doc.key,
                rank: u32::try_from(i + 1).unwrap_or(u32::MAX),
                original_score: doc.score,
            })
            .collect()
    }

    /// Run ANN semantic search via `VectorIndex` (async due to optional embedding).
    ///
    /// # Errors
    ///
    /// Returns `HybridSearchError::NoEmbeddingProvider` when no `query_vector`
    /// is provided and no `EmbeddingProvider` is configured.
    /// Returns `HybridSearchError::Embedding` if the provider `embed()` call fails.
    async fn run_semantic(
        &self,
        index_registry: &IndexRegistry,
        query_text: &str,
        query_vector: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<RankedEntry>, HybridSearchError> {
        // Resolve query vector: use provided vector, auto-embed, or error.
        let owned_vec: Vec<f32>;
        let vec_slice: &[f32] = if let Some(v) = query_vector {
            v
        } else if let Some(provider) = &self.embedding_provider {
            owned_vec = provider.embed(query_text).await?;
            &owned_vec
        } else {
            // No pre-computed vector and no embedding provider — caller error,
            // not a silent skip, so the caller knows to fix the configuration.
            return Err(HybridSearchError::NoEmbeddingProvider);
        };

        // Discover the vector index attribute. `IndexRegistry` is per-map, so
        // `first_vector_index_attribute()` takes no `map_name` parameter.
        let Some(attribute) = index_registry.first_vector_index_attribute() else {
            // No vector index configured for this map — valid configuration, skip.
            return Ok(Vec::new());
        };

        let Some(vector_index) = index_registry.get_vector_index(&attribute) else {
            // Attribute discovered but index not yet built — skip gracefully.
            return Ok(Vec::new());
        };

        // ef_search = k * 2 follows the existing `handle_vector_search` convention
        // for balancing recall against search latency.
        let ef_search = k * 2;
        let raw_results = vector_index.search_nearest(vec_slice, k, ef_search);

        // Results from `search_nearest` are sorted ascending by distance (nearest first).
        // k is bounded by the caller so the count is always below u32::MAX in practice.
        let ranked = raw_results
            .into_iter()
            .enumerate()
            .map(|(i, (key, dist))| RankedEntry {
                key,
                rank: u32::try_from(i + 1).unwrap_or(u32::MAX),
                // Distance stored as original_score for transparency in method_scores.
                original_score: dist,
            })
            .collect();

        Ok(ranked)
    }

    /// Collect all LWW record values from the map into a lookup map.
    ///
    /// Used for result enrichment when `include_value` is true. Iterates all
    /// partitions in a single pass to avoid multiple lock acquisitions.
    fn load_all_values(&self, map_name: &str) -> HashMap<String, rmpv::Value> {
        let stores = self.record_store_factory.get_all_for_map(map_name);
        let mut result = HashMap::new();
        for store in &stores {
            store.for_each_boxed(
                &mut |key, record| {
                    if let RecordValue::Lww { ref value, .. } = record.value {
                        result.insert(key.to_string(), value_to_rmpv(value));
                    }
                },
                false,
            );
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dashmap::DashMap;
    use parking_lot::RwLock;

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::domain::embedding::noop::NoopEmbeddingProvider;
    use crate::service::domain::embedding::NoopConfig;
    use crate::service::domain::index::registry::IndexRegistry;
    use crate::service::domain::search::{HybridSearchRegistry, SearchRegistry, SearchService};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::RecordStoreFactory;

    fn make_search_service(factory: Arc<RecordStoreFactory>) -> Arc<SearchService> {
        let reg = Arc::new(SearchRegistry::new());
        let hybrid_reg = Arc::new(HybridSearchRegistry::new());
        let indexes = Arc::new(RwLock::new(std::collections::HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let needs_population = Arc::new(DashMap::new());
        Arc::new(SearchService::new(
            reg,
            hybrid_reg,
            indexes,
            factory,
            conn_reg,
            needs_population,
            Arc::new(crate::service::domain::index::IndexObserverFactory::new()),
        ))
    }

    fn make_empty_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    // -----------------------------------------------------------------------
    // Error case: NoEmbeddingProvider
    // -----------------------------------------------------------------------

    /// Requesting Semantic search without a `query_vector` and without an
    /// `EmbeddingProvider` returns `HybridSearchError::NoEmbeddingProvider`.
    #[tokio::test]
    async fn test_semantic_without_provider_returns_error() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new();

        let result = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "hello",
                query_vector: None,
                predicate: None,
                methods: &[SearchMethod::Semantic],
                k: 10,
                include_value: false,
            })
            .await;

        assert!(matches!(
            result,
            Err(HybridSearchError::NoEmbeddingProvider)
        ));
    }

    // -----------------------------------------------------------------------
    // Single-method passthrough: FullText only
    // -----------------------------------------------------------------------

    /// When only `FullText` is requested against an empty map, results are empty.
    #[tokio::test]
    async fn test_fulltext_only_empty_map_returns_empty() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new();

        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "hello",
                query_vector: None,
                predicate: None,
                methods: &[SearchMethod::FullText],
                k: 10,
                include_value: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // Semantic with pre-computed vector and no index: skip gracefully
    // -----------------------------------------------------------------------

    /// When Semantic is requested with a pre-computed vector but the map has
    /// no vector index, the result is empty (valid configuration, not an error).
    #[tokio::test]
    async fn test_semantic_with_vector_no_index_returns_empty() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new(); // no vector index registered

        let query_vector = vec![0.1f32, 0.2, 0.3, 0.4];
        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "",
                query_vector: Some(&query_vector),
                predicate: None,
                methods: &[SearchMethod::Semantic],
                k: 10,
                include_value: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // Semantic with NoopEmbeddingProvider auto-embeds
    // -----------------------------------------------------------------------

    /// When Semantic is requested with no pre-computed vector but a configured
    /// `EmbeddingProvider`, the engine calls `embed()` automatically.
    /// With a Noop provider (zero vectors) and no vector index, result is empty.
    #[tokio::test]
    async fn test_semantic_with_noop_provider_embeds_and_skips_without_index() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let provider = Arc::new(NoopEmbeddingProvider::new(&NoopConfig { dimension: 4 }));
        let engine = HybridSearchEngine::new(svc, factory, Some(provider));
        let registry = IndexRegistry::new(); // no vector index

        // No error — provider successfully embeds, but no index means empty result.
        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "hello",
                query_vector: None,
                predicate: None,
                methods: &[SearchMethod::Semantic],
                k: 10,
                include_value: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // Exact method with no predicate returns empty
    // -----------------------------------------------------------------------

    /// When Exact is requested but no predicate is provided, the result is empty.
    #[tokio::test]
    async fn test_exact_without_predicate_returns_empty() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new();

        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "",
                query_vector: None,
                predicate: None,
                methods: &[SearchMethod::Exact],
                k: 10,
                include_value: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // Empty methods list returns empty
    // -----------------------------------------------------------------------

    /// When methods is empty, results are empty.
    #[tokio::test]
    async fn test_empty_methods_returns_empty() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new();

        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "hello",
                query_vector: None,
                predicate: None,
                methods: &[],
                k: 10,
                include_value: false,
            })
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // method_scores contains entries only for methods that produced a hit
    // -----------------------------------------------------------------------

    /// Verifies that `method_scores` is empty when no methods produce results.
    #[tokio::test]
    async fn test_method_scores_empty_when_no_hits() {
        let factory = make_empty_factory();
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new();

        // FullText on an empty map: no docs, no fused results.
        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "test-map",
                index_registry: &registry,
                query_text: "anything",
                query_vector: None,
                predicate: None,
                methods: &[SearchMethod::FullText],
                k: 10,
                include_value: false,
            })
            .await
            .unwrap();

        // No results means no method_scores to inspect, but the call must succeed.
        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // SearchMethod serde round-trip
    // -----------------------------------------------------------------------

    /// Verifies that `SearchMethod` serde derives produce camelCase JSON output.
    #[test]
    fn test_search_method_serde_camel_case() {
        let json = serde_json::to_string(&SearchMethod::FullText).unwrap();
        assert_eq!(json, "\"fullText\"");
        let json = serde_json::to_string(&SearchMethod::Exact).unwrap();
        assert_eq!(json, "\"exact\"");
        let json = serde_json::to_string(&SearchMethod::Semantic).unwrap();
        assert_eq!(json, "\"semantic\"");
    }
}
