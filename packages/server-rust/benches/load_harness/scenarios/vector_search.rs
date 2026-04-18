#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown
)]
/// Vector search benchmark scenario for the `TopGun` load harness.
///
/// Measures HNSW build, query, optimize-cycle, and hybrid search (RRF fusion)
/// performance directly against `Arc<VectorIndex>` — bypassing the WebSocket
/// path so only the HNSW/hybrid-search hot path is captured.
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use tokio::task::JoinSet;
use topgun_core::vector::{DistanceMetric, Vector};

use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::service::domain::index::registry::IndexRegistry;
use topgun_server::service::domain::index::VectorIndex;
use topgun_server::service::domain::search::{
    HybridSearchRegistry, SearchRegistry, SearchService, TantivyMapIndex,
};
use topgun_server::storage::datastores::NullDataStore;
use topgun_server::storage::factory::RecordStoreFactory;
use topgun_server::storage::impls::StorageConfig;

use crate::traits::{Assertion, AssertionResult, HarnessContext, LoadScenario, ScenarioResult};

// ---------------------------------------------------------------------------
// Seeded RNG helpers — deterministic across runs for reproducible baselines
// ---------------------------------------------------------------------------

/// Corpus for Hybrid mode document text. Fixed 8-word list for stable Tantivy builds.
const CORPUS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
];

/// Seed for synthetic vector generation.
const VEC_SEED: u64 = 0xBEEF;

/// Seed for query vector generation (distinct from index vectors to avoid trivial hits).
const QUERY_SEED: u64 = 0xCAFE;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Which vector sub-benchmark to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorMode {
    Build,
    Query,
    Optimize,
    Hybrid,
}

/// Configuration for the vector search scenario.
pub struct VectorSearchConfig {
    /// Sub-mode to run.
    pub mode: VectorMode,
    /// Number of vectors to pre-populate (and build) in the HNSW index.
    pub vector_count: usize,
    /// Embedding dimensionality (must match VectorIndex::new dimension).
    pub vector_dim: u16,
    /// Top-k neighbours to retrieve in Query mode.
    pub top_k: usize,
    /// Wall-clock seconds to run Query / Hybrid modes.
    pub duration_secs: u64,
    /// HNSW ef_search parameter.
    pub ef_search: usize,
    /// Number of concurrent JoinSet tasks for Query mode.
    /// Defaults to available_parallelism (or 4 as fallback).
    pub query_tasks: usize,
}

// ---------------------------------------------------------------------------
// VectorSearchScenario
// ---------------------------------------------------------------------------

/// Load scenario that benchmarks the vector search subsystem.
pub struct VectorSearchScenario {
    config: VectorSearchConfig,
}

impl VectorSearchScenario {
    pub fn new(config: VectorSearchConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl LoadScenario for VectorSearchScenario {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "vector_search"
    }

    async fn setup(&self, _ctx: &HarnessContext) -> Result<()> {
        // Mode-specific setup (pre-population etc.) is performed inside run()
        // so that per-mode state (VectorIndex, TantivyMapIndex, etc.) can be
        // owned across the setup → run boundary without shared-state
        // complexity. Heavyweight pre-population is intentionally excluded from
        // the timed measurement window — see run_*_mode helpers for details.
        Ok(())
    }

    async fn run(&self, ctx: &HarnessContext) -> ScenarioResult {
        match self.config.mode {
            VectorMode::Build => run_build_mode(&self.config, ctx).await,
            VectorMode::Query => run_query_mode(&self.config, ctx).await,
            VectorMode::Optimize => run_optimize_mode(&self.config, ctx).await,
            VectorMode::Hybrid => run_hybrid_mode(&self.config, ctx).await,
        }
    }

    fn assertions(&self) -> Vec<Box<dyn Assertion>> {
        vec![Box::new(VectorSearchAssertion {
            config_mode: self.config.mode,
        })]
    }
}

// ---------------------------------------------------------------------------
// VectorSearchAssertion
// ---------------------------------------------------------------------------

/// Post-run assertion that checks measured p50 against baseline.json thresholds.
pub struct VectorSearchAssertion {
    config_mode: VectorMode,
}

impl VectorSearchAssertion {
    fn mode_key(&self) -> &'static str {
        match self.config_mode {
            VectorMode::Build => "build",
            VectorMode::Query => "query",
            VectorMode::Optimize => "optimize",
            VectorMode::Hybrid => "hybrid",
        }
    }

    fn latency_metric_key(&self) -> &'static str {
        match self.config_mode {
            VectorMode::Build => "vector_build_latency",
            VectorMode::Query => "vector_query_latency",
            VectorMode::Optimize => "vector_optimize_latency",
            VectorMode::Hybrid => "hybrid_search_latency",
        }
    }
}

#[async_trait]
impl Assertion for VectorSearchAssertion {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "vector_search_assertion"
    }

    async fn check(&self, ctx: &HarnessContext, _result: &ScenarioResult) -> AssertionResult {
        // baseline.json is embedded at compile time so the assertion is path-independent
        // and works regardless of the working directory when the bench binary is invoked.
        let baseline_content = include_str!("../baseline.json");

        let Ok(baseline) = serde_json::from_str::<serde_json::Value>(baseline_content) else {
            return AssertionResult::Fail("cannot parse embedded baseline.json".to_string());
        };

        let mode_key = self.mode_key();
        let threshold = baseline
            .get("vector_search")
            .and_then(|vs| vs.get(mode_key))
            .and_then(|m| m.get("max_p50_us"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        // First ever run: no baseline committed yet — pass and note it.
        if threshold == 0 {
            println!("[no baseline for vector-{mode_key}; recording first measurement]");
            return AssertionResult::Pass;
        }

        let snapshot = ctx.metrics.snapshot();
        let latency_key = self.latency_metric_key();
        let Some(stats) = snapshot.latencies.get(latency_key) else {
            return AssertionResult::Fail(format!("no latency data recorded for {latency_key}"));
        };

        if stats.p50 <= threshold {
            AssertionResult::Pass
        } else {
            AssertionResult::Fail(format!(
                "p50 {}µs > baseline {}µs for mode {}",
                stats.p50, threshold, mode_key
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers: synthetic vector generation
// ---------------------------------------------------------------------------

/// Generate `count` synthetic f32 vectors of length `dim`, seeded for determinism.
///
/// Vectors are L2-normalised so cosine distances are meaningful.
fn generate_vectors(count: usize, dim: u16, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = SmallRng::seed_from_u64(seed);
    let d = dim as usize;
    (0..count)
        .map(|_| {
            let mut v: Vec<f32> = (0..d).map(|_| rng.random::<f32>()).collect();
            // L2-normalise for meaningful cosine distances.
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 1e-9 {
                for x in &mut v {
                    *x /= norm;
                }
            }
            v
        })
        .collect()
}

/// Encode a raw `Vec<f32>` into the `rmpv::Value::Map` record format that
/// `VectorIndex::insert` expects: `{ "embedding": Binary(rmp-encoded Vector) }`.
fn encode_vector_record(vec: &[f32]) -> rmpv::Value {
    let v = Vector::F32(vec.to_vec());
    let encoded = rmp_serde::to_vec_named(&v).expect("vector rmp serialization failed");
    rmpv::Value::Map(vec![(
        rmpv::Value::String(rmpv::Utf8String::from("embedding")),
        rmpv::Value::Binary(encoded),
    )])
}

/// Populate `index` with `vectors` in batches of `batch_size`, committing after each batch.
///
/// Commit is wrapped in `spawn_blocking` to match the production callsite pattern.
/// Returns total time for all inserts+commits.
async fn populate_index(index: &Arc<VectorIndex>, vectors: &[Vec<f32>], batch_size: usize) {
    use topgun_server::service::domain::index::Index;
    for (i, chunk) in vectors.chunks(batch_size).enumerate() {
        for (j, vec) in chunk.iter().enumerate() {
            let key = format!("v-{}", i * batch_size + j);
            let record = encode_vector_record(vec);
            index.insert(&key, &record);
        }
        let idx_clone = Arc::clone(index);
        tokio::task::spawn_blocking(move || {
            idx_clone.commit_pending();
        })
        .await
        .expect("commit_pending task panicked");
    }
}

// ---------------------------------------------------------------------------
// Build mode
// ---------------------------------------------------------------------------

async fn run_build_mode(config: &VectorSearchConfig, ctx: &HarnessContext) -> ScenarioResult {
    use topgun_server::service::domain::index::Index;

    let dim = config.vector_dim;
    let count = config.vector_count;
    let batch_size = 1000_usize;

    let index = Arc::new(VectorIndex::new(
        "embedding",
        "bench_vec_idx",
        dim,
        DistanceMetric::Cosine,
        true,
    ));

    let vectors = generate_vectors(count, dim, VEC_SEED);

    println!("Build mode: inserting {count} vectors (dim={dim}) in batches of {batch_size}...");

    let mut total_ops: u64 = 0;
    let start = Instant::now();

    for (i, chunk) in vectors.chunks(batch_size).enumerate() {
        let batch_start = Instant::now();

        for (j, vec) in chunk.iter().enumerate() {
            let key = format!("v-{}", i * batch_size + j);
            let record = encode_vector_record(vec);
            index.insert(&key, &record);
        }

        let idx_clone = Arc::clone(&index);
        tokio::task::spawn_blocking(move || {
            idx_clone.commit_pending();
        })
        .await
        .expect("commit_pending task panicked");

        let elapsed_us = batch_start.elapsed().as_micros() as u64;
        ctx.metrics
            .record_latency("vector_build_latency", elapsed_us);
        total_ops += chunk.len() as u64;
    }

    let duration = start.elapsed();
    println!(
        "Build complete: {} vectors in {:.2}s ({:.0} vectors/sec)",
        total_ops,
        duration.as_secs_f64(),
        total_ops as f64 / duration.as_secs_f64().max(0.001)
    );

    ScenarioResult {
        total_ops,
        duration,
        error_count: 0,
        custom: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Query mode
// ---------------------------------------------------------------------------

async fn run_query_mode(config: &VectorSearchConfig, ctx: &HarnessContext) -> ScenarioResult {
    let dim = config.vector_dim;
    let count = config.vector_count;
    let top_k = config.top_k;
    let ef_search = config.ef_search;
    let duration_secs = config.duration_secs;
    let query_tasks = config.query_tasks;

    println!(
        "Query mode: pre-populating {count} vectors (dim={dim}), then running {query_tasks} concurrent query tasks for {duration_secs}s..."
    );

    // Setup: pre-populate index outside the timed window.
    let index = Arc::new(VectorIndex::new(
        "embedding",
        "bench_vec_idx",
        dim,
        DistanceMetric::Cosine,
        true,
    ));
    let vectors = generate_vectors(count, dim, VEC_SEED);
    populate_index(&index, &vectors, 1000).await;

    println!("Pre-population complete. Starting timed query phase...");

    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let metrics = Arc::clone(&ctx.metrics);

    let mut join_set: JoinSet<u64> = JoinSet::new();

    for task_id in 0..query_tasks {
        let idx_clone = Arc::clone(&index);
        let metrics_clone = Arc::clone(&metrics);
        let deadline_clone = deadline;
        // Each task uses its own RNG seeded from task_id for independence.
        let query_seed = QUERY_SEED.wrapping_add(task_id as u64);

        // spawn_blocking: search_nearest holds an RwLock internally.
        // JoinSet::spawn_blocking runs on the blocking thread pool.
        join_set.spawn_blocking(move || {
            let mut rng = SmallRng::seed_from_u64(query_seed);
            let d = dim as usize;
            let mut task_ops: u64 = 0;

            while Instant::now() < deadline_clone {
                // Generate random query vector and L2-normalise.
                let mut q: Vec<f32> = (0..d).map(|_| rng.random::<f32>()).collect();
                let norm: f32 = q.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 1e-9 {
                    for x in &mut q {
                        *x /= norm;
                    }
                }

                let call_start = Instant::now();
                let _ = idx_clone.search_nearest(&q, top_k, ef_search);
                let elapsed_us = call_start.elapsed().as_micros() as u64;
                metrics_clone.record_latency("vector_query_latency", elapsed_us);
                task_ops += 1;
            }

            task_ops
        });
    }

    let query_phase_start = Instant::now();
    let mut total_ops: u64 = 0;
    while let Some(result) = join_set.join_next().await {
        total_ops += result.expect("query JoinSet task failed");
    }
    let duration = query_phase_start.elapsed();

    println!("Query complete: {total_ops} queries across {query_tasks} tasks");

    ScenarioResult {
        total_ops,
        duration,
        error_count: 0,
        custom: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Optimize mode
// ---------------------------------------------------------------------------

async fn run_optimize_mode(config: &VectorSearchConfig, ctx: &HarnessContext) -> ScenarioResult {
    let dim = config.vector_dim;
    let count = config.vector_count;

    println!(
        "Optimize mode: pre-populating {count} vectors (dim={dim}), then running a single optimize cycle..."
    );

    // Setup: pre-populate outside the timed window.
    let index = Arc::new(VectorIndex::new(
        "embedding",
        "bench_vec_idx",
        dim,
        DistanceMetric::Cosine,
        true,
    ));
    let vectors = generate_vectors(count, dim, VEC_SEED);
    populate_index(&index, &vectors, 1000).await;

    println!("Pre-population complete. Starting optimize...");

    let optimize_start = Instant::now();
    let (handle, _was_already_running) = index.optimize();

    // Poll `finished` every 10ms; record interval duration under
    // `optimize_progress_interval_us` for rebuild smoothness visibility.
    let mut last_sample = Instant::now();
    while !handle.finished.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let now = Instant::now();
        let interval_us = now.duration_since(last_sample).as_micros() as u64;
        ctx.metrics
            .record_latency("optimize_progress_interval_us", interval_us);
        last_sample = now;
    }

    let total_elapsed = optimize_start.elapsed();
    let total_elapsed_us = total_elapsed.as_micros() as u64;
    ctx.metrics
        .record_latency("vector_optimize_latency", total_elapsed_us);

    println!(
        "Optimize complete: {count} vectors rebuilt in {:.2}s",
        total_elapsed.as_secs_f64()
    );

    ScenarioResult {
        total_ops: count as u64,
        duration: total_elapsed,
        error_count: 0,
        custom: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Hybrid mode helpers
// ---------------------------------------------------------------------------

/// Build a minimal `SearchService` following the `make_search_service` pattern
/// from `hybrid.rs` tests (SearchRegistry + HybridSearchRegistry + empty Tantivy
/// index map + ConnectionRegistry + empty DashMap + IndexObserverFactory).
fn make_bench_search_service(
    factory: Arc<RecordStoreFactory>,
    indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>>,
) -> Arc<SearchService> {
    let reg = Arc::new(SearchRegistry::new());
    let hybrid_reg = Arc::new(HybridSearchRegistry::new());
    let conn_reg = Arc::new(ConnectionRegistry::new());
    let needs_population: Arc<DashMap<String, std::sync::atomic::AtomicBool>> =
        Arc::new(DashMap::new());
    Arc::new(SearchService::new(
        reg,
        hybrid_reg,
        indexes,
        factory,
        conn_reg,
        needs_population,
        Arc::new(topgun_server::service::domain::index::IndexObserverFactory::new()),
    ))
}

// ---------------------------------------------------------------------------
// Hybrid mode
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
async fn run_hybrid_mode(config: &VectorSearchConfig, ctx: &HarnessContext) -> ScenarioResult {
    use topgun_server::service::domain::search::hybrid::{
        HybridSearchEngine, HybridSearchParams, SearchMethod as EngineSearchMethod,
    };

    let dim = config.vector_dim;
    let count = config.vector_count;
    let top_k = config.top_k;
    let duration_secs = config.duration_secs;

    println!(
        "Hybrid mode: pre-populating {count} vectors + Tantivy docs (dim={dim}), then running hybrid search for {duration_secs}s..."
    );

    // --- Setup: build VectorIndex ---
    let vector_index = Arc::new(VectorIndex::new(
        "embedding",
        "bench_vec_idx",
        dim,
        DistanceMetric::Cosine,
        true,
    ));
    let vectors = generate_vectors(count, dim, VEC_SEED);
    populate_index(&vector_index, &vectors, 1000).await;

    // --- Setup: build Tantivy index for the bench map ---
    let bench_map_name = "bench_map";
    let tantivy_idx = TantivyMapIndex::new();

    let mut rng = SmallRng::seed_from_u64(VEC_SEED);
    for i in 0..count {
        let word = CORPUS[rng.random_range(0..CORPUS.len())];
        let doc_text = format!("doc {i} about topic {word}");
        // Build rmpv record with a single "_all_text"-equivalent field.
        let record = rmpv::Value::Map(vec![(
            rmpv::Value::String(rmpv::Utf8String::from("text")),
            rmpv::Value::String(rmpv::Utf8String::from(doc_text.clone())),
        )]);
        tantivy_idx.index_document(&format!("doc-{i}"), &record);
    }
    // commit() is CPU-bound but brief for 10K docs; call synchronously in setup
    // (not inside the timed measurement window) to avoid the unsafe-block constraint.
    tantivy_idx.commit();

    // Wrap in the Arc<RwLock<HashMap>> that SearchService holds.
    let indexes: Arc<RwLock<HashMap<String, TantivyMapIndex>>> = Arc::new(RwLock::new({
        let mut m = HashMap::new();
        m.insert(bench_map_name.to_string(), tantivy_idx);
        m
    }));

    // --- Setup: minimal SearchService and HybridSearchEngine ---
    let factory = Arc::new(RecordStoreFactory::new(
        StorageConfig::default(),
        Arc::new(NullDataStore),
        Vec::new(),
    ));
    let search_service = make_bench_search_service(Arc::clone(&factory), Arc::clone(&indexes));

    // --- Setup: IndexRegistry with the VectorIndex registered ---
    // IndexRegistry::add_vector_index_with_params creates its own Arc<VectorIndex>.
    // We populate the registry-owned index via the Index trait (same as populate_index
    // helper) so that HybridSearchEngine.run_semantic() finds a pre-populated index
    // via index_registry.get_vector_index("embedding").
    let index_registry = Arc::new(IndexRegistry::new());
    let registry_vector_index = index_registry.add_vector_index_with_params(
        "embedding",
        "bench_vec_idx",
        dim,
        DistanceMetric::Cosine,
        16,
        200,
        true,
    );
    // Populate the registry's vector index.
    populate_index(&registry_vector_index, &vectors, 1000).await;

    let engine = Arc::new(HybridSearchEngine::new(
        Arc::clone(&search_service),
        Arc::clone(&factory),
        None,
    ));

    println!("Hybrid setup complete. Starting timed hybrid search phase...");

    // --- Timed run ---
    let bench_map_str = bench_map_name.to_string();
    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let mut total_ops: u64 = 0;
    let start = Instant::now();
    let mut query_rng = SmallRng::seed_from_u64(QUERY_SEED);
    let d = dim as usize;

    let methods: &[EngineSearchMethod] = &[
        EngineSearchMethod::Exact,
        EngineSearchMethod::FullText,
        EngineSearchMethod::Semantic,
    ];

    while Instant::now() < deadline {
        // Owned data for this iteration — must outlive the borrow in HybridSearchParams.
        let query_text = CORPUS[query_rng.random_range(0..CORPUS.len())].to_string();
        let mut qv: Vec<f32> = (0..d).map(|_| query_rng.random::<f32>()).collect();
        let norm: f32 = qv.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-9 {
            for x in &mut qv {
                *x /= norm;
            }
        }

        let call_start = Instant::now();
        let params = HybridSearchParams {
            map_name: bench_map_str.as_str(),
            index_registry: &index_registry,
            query_text: &query_text,
            query_vector: Some(&qv),
            predicate: None,
            methods,
            k: top_k,
            include_value: false,
        };

        // hybrid_search is async (runs semantic via blocking task internally).
        let _ = engine.hybrid_search(params).await;
        let elapsed_us = call_start.elapsed().as_micros() as u64;
        ctx.metrics
            .record_latency("hybrid_search_latency", elapsed_us);
        total_ops += 1;
    }

    let duration = start.elapsed();
    println!(
        "Hybrid complete: {total_ops} queries in {:.2}s",
        duration.as_secs_f64()
    );

    ScenarioResult {
        total_ops,
        duration,
        error_count: 0,
        custom: HashMap::new(),
    }
}
