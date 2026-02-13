use async_trait::async_trait;

use crate::types::Value;

/// Context provided to a Processor vertex on initialization.
#[derive(Debug)]
pub struct ProcessorContext {
    /// Zero-based index of this vertex within the DAG stage.
    pub vertex_index: usize,
    /// Total number of parallel vertices in this stage.
    pub total_parallelism: usize,
}

/// Inbound message queue for a Processor vertex.
/// Placeholder: will carry typed messages with backpressure semantics when DAG executor is built.
#[derive(Debug)]
pub struct Inbox {
    /// Index of the inbound edge this inbox receives from.
    pub ordinal: usize,
}

/// Vertex in a DAG execution graph (Hazelcast-style distributed query processing).
/// Each vertex receives items from an Inbox, processes them, and signals completion.
#[async_trait]
pub trait Processor: Send {
    /// One-time initialization with execution context.
    async fn init(&mut self, ctx: ProcessorContext) -> anyhow::Result<()>;

    /// Process a batch from the inbox. Returns true when this ordinal is fully processed.
    async fn process(&mut self, ordinal: usize, inbox: &mut Inbox) -> anyhow::Result<bool>;

    /// Called after all ordinals are processed. Returns true if complete.
    async fn complete(&mut self) -> anyhow::Result<bool>;

    /// Whether this processor yields cooperatively (affects scheduling).
    fn is_cooperative(&self) -> bool;

    /// Release resources.
    async fn close(&mut self) -> anyhow::Result<()>;
}

/// Write-path notification for live query updates.
/// Implementations observe all writes and notify active query subscriptions.
pub trait QueryNotifier: Send + Sync {
    /// Called on every write. `old_value` enables delta-based optimizations.
    fn notify_change(
        &self,
        map_name: &str,
        key: &str,
        old_value: Option<&Value>,
        new_value: &Value,
    );
}
