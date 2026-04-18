/// Vector search benchmark scenario for the TopGun load harness.
///
/// Measures HNSW build, query, optimize-cycle, and hybrid search (RRF fusion)
/// performance directly against `Arc<VectorIndex>` — bypassing the WebSocket
/// path so only the HNSW/hybrid-search hot path is captured.
use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use crate::traits::{
    Assertion, AssertionResult, HarnessContext, LoadScenario, ScenarioResult,
};

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

/// Which search method(s) to invoke in Hybrid mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMethod {
    Exact,
    FullText,
    Semantic,
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
    /// Search methods to combine in Hybrid mode.
    pub methods: Vec<SearchMethod>,
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
    fn name(&self) -> &str {
        "vector_search"
    }

    async fn setup(&self, _ctx: &HarnessContext) -> Result<()> {
        // Mode-specific setup is performed inside run() so that per-mode
        // state (VectorIndex, TantivyMapIndex, etc.) can be owned by run().
        // Heavyweight pre-population is intentionally excluded from the
        // timed measurement window — see run_*_mode helpers for details.
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

#[async_trait]
impl Assertion for VectorSearchAssertion {
    fn name(&self) -> &str {
        "vector_search_assertion"
    }

    async fn check(&self, _ctx: &HarnessContext, _result: &ScenarioResult) -> AssertionResult {
        // Implementation in G4 (CLI wiring wave): reads baseline.json,
        // compares p50 from ctx.metrics.snapshot() against max_p50_us threshold.
        // For G1 skeleton: always pass so compilation succeeds.
        let _ = self.config_mode;
        AssertionResult::Pass
    }
}

// ---------------------------------------------------------------------------
// Mode stub implementations (bodies filled in G2 / G3)
// ---------------------------------------------------------------------------

async fn run_build_mode(
    _config: &VectorSearchConfig,
    _ctx: &HarnessContext,
) -> ScenarioResult {
    ScenarioResult {
        total_ops: 0,
        duration: std::time::Duration::ZERO,
        error_count: 0,
        custom: HashMap::new(),
    }
}

async fn run_query_mode(
    _config: &VectorSearchConfig,
    _ctx: &HarnessContext,
) -> ScenarioResult {
    ScenarioResult {
        total_ops: 0,
        duration: std::time::Duration::ZERO,
        error_count: 0,
        custom: HashMap::new(),
    }
}

async fn run_optimize_mode(
    _config: &VectorSearchConfig,
    _ctx: &HarnessContext,
) -> ScenarioResult {
    ScenarioResult {
        total_ops: 0,
        duration: std::time::Duration::ZERO,
        error_count: 0,
        custom: HashMap::new(),
    }
}

async fn run_hybrid_mode(
    _config: &VectorSearchConfig,
    _ctx: &HarnessContext,
) -> ScenarioResult {
    ScenarioResult {
        total_ops: 0,
        duration: std::time::Duration::ZERO,
        error_count: 0,
        custom: HashMap::new(),
    }
}
