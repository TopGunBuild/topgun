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
use topgun_core::hash_to_partition;

use crate::service::domain::index::registry::IndexRegistry;
use crate::service::domain::predicate::{evaluate_predicate, value_to_rmpv, EvalContext};
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
    /// A read of the stored map failed: the store scan of `exact`, or the
    /// population the `fullText` leg waited for. Never answered as an empty
    /// result list (`TG-READ-001`).
    #[error("store scan failed: {0}")]
    Store(String),
}

/// Whether the `fullText` leg may start or join a population of the map's
/// full-text index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullTextMode {
    /// A request path: a map that needs population is populated first, and the
    /// leg waits for it.
    Populate,
    /// A hybrid notifier pass: the map's current index is read as it is, and no
    /// population is started or joined.
    CurrentIndex,
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
    /// `exact` scans the stored map (`TG-READ-001`) and `fullText` populates the
    /// map's full-text index first when it needs population. No lock guard is
    /// held across an await.
    ///
    /// # Errors
    ///
    /// Returns `HybridSearchError::Store` when the `exact` scan or the
    /// population the `fullText` leg waited for failed. Returns
    /// `HybridSearchError::NoEmbeddingProvider` when `Semantic` is in
    /// `methods`, no `query_vector` is provided, and no `EmbeddingProvider` is
    /// configured. Returns `HybridSearchError::Embedding` if the provider call
    /// fails.
    pub async fn hybrid_search(
        &self,
        params: HybridSearchParams<'_>,
    ) -> Result<Vec<HybridSearchResult>, HybridSearchError> {
        self.hybrid_search_with_exact(params, None, FullTextMode::Populate)
            .await
    }

    /// Like [`hybrid_search`](Self::hybrid_search), but the `exact` leg takes
    /// `exact` when it is `Some` instead of scanning the map, and the
    /// `fullText` leg runs in `mode`.
    ///
    /// # Errors
    ///
    /// As [`hybrid_search`](Self::hybrid_search).
    pub(crate) async fn hybrid_search_with_exact(
        &self,
        params: HybridSearchParams<'_>,
        exact: Option<Vec<String>>,
        mode: FullTextMode,
    ) -> Result<Vec<HybridSearchResult>, HybridSearchError> {
        let mut ranked_lists: Vec<Vec<RankedEntry>> = Vec::new();
        // Track which list index corresponds to which method for method_scores mapping.
        let mut method_order: Vec<SearchMethod> = Vec::new();

        // --- Exact-match ---
        if params.methods.contains(&SearchMethod::Exact) {
            let keys = match (exact, params.predicate) {
                (Some(keys), _) => keys,
                (None, Some(predicate)) => self
                    .exact_matches(params.map_name, &[predicate])
                    .await?
                    .pop()
                    .unwrap_or_default(),
                (None, None) => Vec::new(),
            };
            // Binary relevance: all matches receive rank 1, original_score = 1.0.
            // RRF handles uniform rank correctly — contribution is 1/(rrf_k+1) each.
            let exact_list: Vec<RankedEntry> = keys
                .into_iter()
                .map(|key| RankedEntry {
                    key,
                    rank: 1,
                    original_score: 1.0,
                })
                .collect();
            if !exact_list.is_empty() {
                ranked_lists.push(exact_list);
                method_order.push(SearchMethod::Exact);
            }
        }

        // --- Full-text BM25 ---
        if params.methods.contains(&SearchMethod::FullText) {
            let ft_list = self
                .run_fulltext(params.map_name, params.query_text, mode)
                .await?;
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

        // --- Map FusedEntry to HybridSearchResult ---
        let mut results = Vec::with_capacity(fused.len());
        for fe in fused {
            // Point reads of the fused keys only, at most `k` of them.
            let value = if params.include_value {
                self.load_value(params.map_name, &fe.key).await
            } else {
                None
            };

            // Translate list indices back to SearchMethod keys for transparency.
            let method_scores: HashMap<SearchMethod, f64> = fe
                .method_scores
                .iter()
                .filter_map(|(list_idx, score)| method_order.get(*list_idx).map(|m| (*m, *score)))
                .collect();

            results.push(HybridSearchResult {
                key: fe.key,
                score: fe.score,
                method_scores,
                value,
            });
        }

        Ok(results)
    }

    /// The keys of `map_name` whose stored or resident `Lww` value matches each
    /// of `predicates`, one list per predicate, from one `scan_merged` of the
    /// map (`TG-READ-001`). No scalar index is consulted, so an index that
    /// misses a key cannot drop a match.
    ///
    /// # Errors
    ///
    /// `HybridSearchError::Store` when the scan fails; never a partial list.
    pub(crate) async fn exact_matches(
        &self,
        map_name: &str,
        predicates: &[&PredicateNode],
    ) -> Result<Vec<Vec<String>>, HybridSearchError> {
        let mut lists: Vec<Vec<String>> = vec![Vec::new(); predicates.len()];
        self.record_store_factory
            .scan_merged(map_name, &mut |key, value| {
                if let RecordValue::Lww { value, .. } = value {
                    let row = value_to_rmpv(value);
                    let ctx = EvalContext::data_only(&row);
                    for (list, predicate) in lists.iter_mut().zip(predicates) {
                        if evaluate_predicate(predicate, &ctx) {
                            list.push(key.to_string());
                        }
                    }
                }
            })
            .await
            .map_err(|error| HybridSearchError::Store(error.to_string()))?;
        // Every match ranks equally; a stable order makes the `k` cut-off of the
        // fusion deterministic.
        for list in &mut lists {
            list.sort_unstable();
        }
        Ok(lists)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Run BM25 full-text search via tantivy `SearchService` accessor.
    async fn run_fulltext(
        &self,
        map_name: &str,
        query_text: &str,
        mode: FullTextMode,
    ) -> Result<Vec<RankedEntry>, HybridSearchError> {
        if query_text.is_empty() {
            return Ok(Vec::new());
        }

        let options = SearchOptions::default();
        let scored_docs = self
            .search_service
            .search_map(map_name, query_text, &options, mode)
            .await
            .map_err(|error| HybridSearchError::Store(error.to_string()))?;

        // Convert score-ordered `ScoredDoc` list to 1-based `RankedEntry` list.
        // Result counts from tantivy are always far below u32::MAX in practice.
        Ok(scored_docs
            .into_iter()
            .enumerate()
            .map(|(i, doc)| RankedEntry {
                key: doc.key,
                rank: u32::try_from(i + 1).unwrap_or(u32::MAX),
                original_score: doc.score,
            })
            .collect())
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

    /// The LWW value of one result key, by the ordinary point read: it loads
    /// the key into its store, cost-accounted and evictable (`TG-EVI-001`). A
    /// read error leaves the value out.
    async fn load_value(&self, map_name: &str, key: &str) -> Option<rmpv::Value> {
        let store = self
            .record_store_factory
            .get_or_create(map_name, hash_to_partition(key));
        match store.get(key, false).await {
            Ok(Some(record)) => match record.value {
                RecordValue::Lww { value, .. } => Some(value_to_rmpv(&value)),
                _ => None,
            },
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(map = %map_name, key, error = %error, "hybrid search: value read failed");
                None
            }
        }
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

    // -----------------------------------------------------------------------
    // TG-READ-001: exact never trusts a scalar index as complete
    // -----------------------------------------------------------------------

    /// AC20: a hash index on `status` that misses key `k`, while `k` with
    /// `status: 'open'` is only in the data store: `exact` still returns `k`.
    #[tokio::test(flavor = "multi_thread")]
    async fn tg_read_001_exact_returns_a_stored_key_the_index_misses() {
        use topgun_core::messages::base::PredicateOp;

        use crate::storage::factory::test_store::TestStore;
        use crate::storage::MapDataStore;

        let store = Arc::new(TestStore::new(100));
        store
            .add(
                "m",
                "k",
                &RecordValue::Lww {
                    value: topgun_core::types::Value::Map(std::collections::BTreeMap::from([(
                        "status".to_string(),
                        topgun_core::types::Value::String("open".to_string()),
                    )])),
                    timestamp: topgun_core::hlc::Timestamp {
                        millis: 1_000,
                        counter: 0,
                        node_id: "n".to_string(),
                    },
                    ttl_ms: None,
                },
                0,
                1_000,
            )
            .await
            .unwrap();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            store,
            Vec::new(),
        ));
        let svc = make_search_service(Arc::clone(&factory));
        let engine = HybridSearchEngine::new(svc, factory, None);
        let registry = IndexRegistry::new();
        registry.add_hash_index("status");
        let predicate = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("status".to_string()),
            value: Some(rmpv::Value::String("open".into())),
            ..PredicateNode::default()
        };

        let results = engine
            .hybrid_search(HybridSearchParams {
                map_name: "m",
                index_registry: &registry,
                query_text: "",
                query_vector: None,
                predicate: Some(&predicate),
                methods: &[SearchMethod::Exact],
                k: 10,
                include_value: true,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].key, "k");
        assert!(
            results[0].value.is_some(),
            "includeValue reads the stored value"
        );
    }
}
