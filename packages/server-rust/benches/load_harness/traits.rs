use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use anyhow::Result;

/// All latency percentile statistics for a single operation type.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LatencyStats {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    pub count: u64,
}

/// A point-in-time snapshot of all recorded metrics.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub latencies: HashMap<String, LatencyStats>,
    #[allow(dead_code)]
    pub counters: HashMap<String, u64>,
}

/// Thread-safe metrics recording interface.
pub trait MetricsCollector: Send + Sync {
    fn record_latency(&self, operation: &str, duration_us: u64);
    #[allow(dead_code)]
    fn increment_counter(&self, name: &str, count: u64);
    fn snapshot(&self) -> MetricsSnapshot;
}

/// Shared runtime context passed to every scenario and assertion.
pub struct HarnessContext {
    pub server_addr: SocketAddr,
    pub jwt_secret: String,
    pub metrics: Arc<dyn MetricsCollector>,
    /// Placeholder for the connection pool added in SPEC-121b.
    #[allow(dead_code)]
    pub pool: Option<()>,
}

/// Aggregated results produced by a single scenario run.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScenarioResult {
    pub total_ops: u64,
    pub duration: Duration,
    pub error_count: u64,
    /// Scenario-specific numeric values (e.g. throughput rates, custom counters).
    pub custom: HashMap<String, f64>,
}

/// Outcome of a single post-run assertion.
#[derive(Debug, Clone)]
pub enum AssertionResult {
    Pass,
    Fail(String),
}

/// A load scenario that can set up state, execute operations, and declare assertions.
#[async_trait]
pub trait LoadScenario: Send + Sync {
    fn name(&self) -> &str;
    async fn setup(&self, ctx: &HarnessContext) -> Result<()>;
    async fn run(&self, ctx: &HarnessContext) -> ScenarioResult;
    fn assertions(&self) -> Vec<Box<dyn Assertion>>;
}

/// A post-run correctness check that validates scenario results against collected metrics.
#[async_trait]
pub trait Assertion: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self, ctx: &HarnessContext, result: &ScenarioResult) -> AssertionResult;
}
