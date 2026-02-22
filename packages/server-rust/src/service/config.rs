use topgun_core::PARTITION_COUNT;

/// Server-level configuration for the operation routing framework.
///
/// Controls operation timeouts, concurrency limits, and background task intervals.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Unique identifier for this server node.
    pub node_id: String,
    /// Default timeout for operations in milliseconds.
    pub default_operation_timeout_ms: u64,
    /// Maximum number of concurrent operations before load shedding.
    pub max_concurrent_operations: u32,
    /// Interval between garbage collection runs in milliseconds.
    pub gc_interval_ms: u64,
    /// Number of partitions. Configurable for testing; defaults to
    /// `topgun_core::PARTITION_COUNT` (271) in production.
    pub partition_count: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            node_id: String::new(),
            default_operation_timeout_ms: 30_000,
            max_concurrent_operations: 1000,
            gc_interval_ms: 60_000,
            partition_count: PARTITION_COUNT,
        }
    }
}
