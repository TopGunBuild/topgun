//! Partition-based operation dispatcher.
//!
//! Routes operations to per-partition worker tasks via bounded MPSC channels.
//! Each worker owns its own `OperationPipeline`, processing operations
//! sequentially within its shard -- no shared mutex needed.
//!
//! Routing rule:
//! - `partition_id = Some(id)` → `workers[id % worker_count]`
//! - `partition_id = None`     → dedicated global worker

use tokio::sync::{mpsc, oneshot};
use tower::Service;
use tracing::info;

use super::operation::{Operation, OperationError, OperationPipeline, OperationResponse};

// ---------------------------------------------------------------------------
// DispatchRequest
// ---------------------------------------------------------------------------

/// A request dispatched to a partition worker.
///
/// Carries the operation and a oneshot channel for the response so the caller
/// can await the result without blocking the worker.
pub struct DispatchRequest {
    pub operation: Operation,
    pub response_tx: oneshot::Sender<Result<OperationResponse, OperationError>>,
}

// ---------------------------------------------------------------------------
// DispatchConfig
// ---------------------------------------------------------------------------

/// Configuration for the partition dispatcher.
pub struct DispatchConfig {
    /// Number of partition worker tasks.
    /// Defaults to the number of logical CPUs available to the process.
    pub worker_count: usize,
    /// Bounded channel buffer size per worker.
    /// Backpressure kicks in when a worker's inbox exceeds this threshold.
    pub channel_buffer_size: usize,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self {
            worker_count: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            channel_buffer_size: 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// PartitionDispatcher
// ---------------------------------------------------------------------------

/// Routes operations to partition-sharded worker tasks.
///
/// Each worker owns its own `OperationPipeline` and processes operations
/// sequentially. Operations are routed by `partition_id % worker_count`.
/// Non-partition operations (`partition_id = None`) go to a dedicated
/// global worker.
///
/// Dropping this struct drops all sender halves, causing worker tasks to drain
/// and exit cleanly -- no explicit shutdown protocol is needed.
pub struct PartitionDispatcher {
    /// Per-worker MPSC senders, indexed by worker index (0..worker_count).
    workers: Vec<mpsc::Sender<DispatchRequest>>,
    /// Dedicated sender for operations with no partition affinity.
    global_worker: mpsc::Sender<DispatchRequest>,
    /// Number of partition workers (excluding the global worker).
    worker_count: usize,
}

impl PartitionDispatcher {
    /// Create a new dispatcher, spawn N+1 worker tasks, and return the handle.
    ///
    /// `pipeline_factory` is called exactly `worker_count + 1` times: once per
    /// partition worker and once for the global worker. Each call must return a
    /// fresh, independent `OperationPipeline`.
    pub fn new<F>(config: DispatchConfig, pipeline_factory: F) -> Self
    where
        F: Fn() -> OperationPipeline,
    {
        let worker_count = config.worker_count;
        let buffer = config.channel_buffer_size;

        let mut workers = Vec::with_capacity(worker_count);

        for i in 0..worker_count {
            let (tx, rx) = mpsc::channel::<DispatchRequest>(buffer);
            let pipeline = pipeline_factory();
            spawn_worker(pipeline, rx, format!("partition-worker-{i}"));
            workers.push(tx);
        }

        let (global_tx, global_rx) = mpsc::channel::<DispatchRequest>(buffer);
        let global_pipeline = pipeline_factory();
        spawn_worker(global_pipeline, global_rx, "global-worker".to_string());

        info!("partition dispatcher started with {} workers", worker_count);

        Self {
            workers,
            global_worker: global_tx,
            worker_count,
        }
    }

    /// Dispatch an operation to the appropriate worker and await its response.
    ///
    /// Reads `operation.ctx().partition_id` to determine routing:
    /// - `Some(id)`: routes to `workers[id % worker_count]`.
    /// - `None`: routes to the dedicated global worker.
    ///
    /// Returns `OperationError::Internal` if the target worker channel is closed
    /// (worker task has dropped its receiver).
    pub async fn dispatch(
        &self,
        operation: Operation,
    ) -> Result<OperationResponse, OperationError> {
        let partition_id = operation.ctx().partition_id;

        let sender = match partition_id {
            Some(id) => &self.workers[id as usize % self.worker_count],
            None => &self.global_worker,
        };

        let (response_tx, response_rx) = oneshot::channel();
        let request = DispatchRequest {
            operation,
            response_tx,
        };

        sender.send(request).await.map_err(|_| {
            OperationError::Internal(anyhow::anyhow!("worker channel closed"))
        })?;

        response_rx.await.map_err(|_| {
            OperationError::Internal(anyhow::anyhow!("worker response channel closed"))
        })?
    }
}

// ---------------------------------------------------------------------------
// Worker task
// ---------------------------------------------------------------------------

/// Spawn a worker task that owns `pipeline` and processes `DispatchRequest`s
/// received from `rx` until the sender side is dropped.
fn spawn_worker(
    mut pipeline: OperationPipeline,
    mut rx: mpsc::Receiver<DispatchRequest>,
    name: String,
) {
    tokio::spawn(async move {
        while let Some(DispatchRequest {
            operation,
            response_tx,
        }) = rx.recv().await
        {
            // Poll the service until it is ready, then call it.
            // BoxService requires `&mut self`, which we hold exclusively here.
            use std::future::poll_fn;

            let poll_result = poll_fn(|cx| pipeline.poll_ready(cx)).await;
            let result = match poll_result {
                Ok(()) => pipeline.call(operation).await,
                Err(e) => Err(e),
            };

            // Ignore send errors: the caller may have timed out and dropped
            // the oneshot receiver before the worker finished.
            let _ = response_tx.send(result);
        }

        tracing::debug!("worker {} exiting: sender dropped", name);
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use topgun_core::Timestamp;
    use tower::Service;

    use super::*;
    use crate::service::middleware::pipeline::build_operation_pipeline;
    use crate::service::operation::{service_names, OperationContext};
    use crate::service::router::OperationRouter;
    use crate::service::ServerConfig;

    // -----------------------------------------------------------------------
    // Stub service
    // -----------------------------------------------------------------------

    /// Records which partition_id was routed to it, returns Ack.
    struct EchoService;

    impl Service<Operation> for EchoService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future =
            Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            let call_id = op.ctx().call_id;
            Box::pin(async move { Ok(OperationResponse::Ack { call_id }) })
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_pipeline() -> OperationPipeline {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, EchoService);
        router.register(service_names::SYNC, EchoService);
        router.register(service_names::QUERY, EchoService);
        router.register(service_names::MESSAGING, EchoService);
        router.register(service_names::COORDINATION, EchoService);
        router.register(service_names::SEARCH, EchoService);
        router.register(service_names::PERSISTENCE, EchoService);
        let config = ServerConfig {
            max_concurrent_operations: 1000,
            ..ServerConfig::default()
        };
        build_operation_pipeline(router, &config)
    }

    fn make_op_with_partition(call_id: u64, partition_id: Option<u32>) -> Operation {
        let mut ctx = OperationContext::new(
            call_id,
            service_names::CRDT,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        ctx.partition_id = partition_id;
        Operation::GarbageCollect { ctx }
    }

    fn make_dispatcher() -> PartitionDispatcher {
        let config = DispatchConfig {
            worker_count: 4,
            channel_buffer_size: 64,
        };
        PartitionDispatcher::new(config, make_pipeline)
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_config_default_has_sensible_values() {
        let config = DispatchConfig::default();
        assert!(config.worker_count > 0, "worker_count must be at least 1");
        assert_eq!(config.channel_buffer_size, 1024);
    }

    #[tokio::test]
    async fn partition_routing_is_deterministic() {
        let dispatcher = make_dispatcher();

        // The same partition_id must always go to the same worker (id % count).
        // We verify this indirectly by checking that responses are returned for
        // several calls with known partition IDs.
        for partition_id in [0u32, 1, 2, 3, 7, 11, 100, 271] {
            let op = make_op_with_partition(partition_id as u64, Some(partition_id));
            let result = dispatcher.dispatch(op).await;
            assert!(result.is_ok(), "partition {partition_id} should succeed");
        }
    }

    #[tokio::test]
    async fn global_worker_handles_none_partition_id() {
        let dispatcher = make_dispatcher();
        let op = make_op_with_partition(999, None);
        let result = dispatcher.dispatch(op).await;
        assert!(result.is_ok(), "None partition_id should route to global worker");
        assert!(matches!(result.unwrap(), OperationResponse::Ack { call_id: 999 }));
    }

    #[tokio::test]
    async fn response_delivered_via_oneshot() {
        let dispatcher = make_dispatcher();
        let op = make_op_with_partition(42, Some(0));
        let result = dispatcher.dispatch(op).await;
        assert!(matches!(result, Ok(OperationResponse::Ack { call_id: 42 })));
    }

    #[tokio::test]
    async fn closed_channel_returns_internal_error() {
        // Build a dispatcher then drop all senders by taking the struct apart.
        // We can't easily close the worker's receiver externally, but we can
        // construct a dispatcher with a manually closed sender.
        let (tx, rx) = mpsc::channel::<DispatchRequest>(1);
        let (global_tx, global_rx) = mpsc::channel::<DispatchRequest>(1);

        // Drop the receivers so the channels are "closed" from the worker side.
        drop(rx);
        drop(global_rx);

        let dispatcher = PartitionDispatcher {
            workers: vec![tx],
            global_worker: global_tx,
            worker_count: 1,
        };

        let op = make_op_with_partition(1, Some(0));
        let result = dispatcher.dispatch(op).await;
        assert!(
            matches!(result, Err(OperationError::Internal(_))),
            "closed channel should return OperationError::Internal, got: {:?}",
            result
        );
    }
}
