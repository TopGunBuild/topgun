//! Cluster message dispatch loop.
//!
//! Consumes inbound binary frames from cluster peer connections, deserializes
//! them as [`ClusterMessage`], and routes DAG variants to the handlers in
//! [`crate::service::domain::dag_dispatch`].

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};

use crate::cluster::messages::ClusterMessage;
use crate::cluster::state::InboundClusterMessage;
use crate::network::connection::ConnectionRegistry;
use crate::service::domain::dag_dispatch::{
    handle_dag_complete, handle_dag_data, handle_dag_execute,
};
use crate::storage::RecordStoreFactory;

use crate::cluster::messages::DagCompletePayload;

// ---------------------------------------------------------------------------
// ClusterDispatchContext
// ---------------------------------------------------------------------------

/// Holds all dependencies the cluster dispatch loop needs.
///
/// All fields are `Arc`-wrapped so the struct is cheap to clone when spawning
/// per-message tasks (e.g., `handle_dag_execute` is spawned on a new tokio task
/// because it performs async I/O).
#[derive(Clone)]
pub struct ClusterDispatchContext {
    pub local_node_id: String,
    pub completion_registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>>,
    pub record_store_factory: Arc<RecordStoreFactory>,
    pub connection_registry: Arc<ConnectionRegistry>,
}

// ---------------------------------------------------------------------------
// HandleFrameError
// ---------------------------------------------------------------------------

/// Errors returned by [`handle_cluster_peer_frame`].
#[derive(Debug)]
pub enum HandleFrameError {
    /// The binary frame could not be deserialized as a `ClusterMessage`.
    Deserialize(rmp_serde::decode::Error),
    /// The inbound channel is full or closed.
    ChannelClosed,
}

// ---------------------------------------------------------------------------
// run_cluster_dispatch_loop
// ---------------------------------------------------------------------------

/// Reads `InboundClusterMessage` items from `rx` and routes them to the
/// appropriate handler based on the `ClusterMessage` variant.
///
/// Exits cleanly when `rx` is closed (all senders dropped). The caller is
/// responsible for wrapping this in `tokio::spawn` and holding the resulting
/// `JoinHandle` for shutdown.
pub async fn run_cluster_dispatch_loop(
    ctx: ClusterDispatchContext,
    mut rx: mpsc::Receiver<InboundClusterMessage>,
) {
    while let Some(inbound) = rx.recv().await {
        let InboundClusterMessage {
            sender_node_id,
            message,
        } = inbound;

        match message {
            ClusterMessage::DagExecute(payload) => {
                let ctx = ctx.clone();
                let sender = sender_node_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_dag_execute(
                        payload,
                        &ctx.local_node_id,
                        Arc::clone(&ctx.record_store_factory),
                        &sender,
                        Arc::clone(&ctx.connection_registry),
                    )
                    .await
                    {
                        tracing::warn!(
                            sender_node_id = %sender,
                            error = %e,
                            "handle_dag_execute failed"
                        );
                    }
                });
            }
            ClusterMessage::DagComplete(payload) => {
                handle_dag_complete(payload, &ctx.completion_registry);
            }
            ClusterMessage::DagData(payload) => {
                handle_dag_data(payload);
            }
            other => {
                tracing::debug!(
                    variant = std::any::type_name_of_val(&other),
                    sender_node_id = %sender_node_id,
                    "cluster message received but no handler wired yet"
                );
            }
        }
    }

    tracing::debug!("cluster dispatch loop exiting: channel closed");
}

// ---------------------------------------------------------------------------
// handle_cluster_peer_frame
// ---------------------------------------------------------------------------

/// Deserializes a raw binary frame as a [`ClusterMessage`] and sends it into
/// the inbound message channel for the dispatch loop to process.
///
/// This is a synchronous function (`try_send`) because the sim transport calls
/// it from a synchronous context. Production WebSocket handler wiring is a
/// future spec.
///
/// # Errors
///
/// Returns [`HandleFrameError::Deserialize`] if the bytes cannot be decoded as
/// a `ClusterMessage`, or [`HandleFrameError::ChannelClosed`] if the inbound
/// channel is full or closed.
pub fn handle_cluster_peer_frame(
    bytes: &[u8],
    sender_node_id: String,
    tx: &mpsc::Sender<InboundClusterMessage>,
) -> Result<(), HandleFrameError> {
    let message: ClusterMessage = rmp_serde::from_slice(bytes).map_err(|e| {
        tracing::warn!(sender_node_id = %sender_node_id, error = %e, "failed to deserialize cluster peer frame");
        HandleFrameError::Deserialize(e)
    })?;

    let inbound = InboundClusterMessage {
        sender_node_id,
        message,
    };

    tx.try_send(inbound)
        .map_err(|_| HandleFrameError::ChannelClosed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use dashmap::DashMap;
    use tokio::sync::{mpsc, oneshot};

    use crate::cluster::messages::{ClusterMessage, DagCompletePayload, HeartbeatPayload};
    use crate::network::connection::ConnectionRegistry;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;

    use super::{
        handle_cluster_peer_frame, run_cluster_dispatch_loop, ClusterDispatchContext,
        HandleFrameError,
    };

    fn make_ctx(
        completion_registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>>,
    ) -> ClusterDispatchContext {
        ClusterDispatchContext {
            local_node_id: "test-node".to_string(),
            completion_registry,
            record_store_factory: Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )),
            connection_registry: Arc::new(ConnectionRegistry::new()),
        }
    }

    // -- AC1: DagComplete bytes routed through dispatch loop resolve completion_registry --

    #[tokio::test]
    async fn dag_complete_routed_resolves_completion_registry() {
        let registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>> =
            Arc::new(DashMap::new());

        let (tx, rx) = mpsc::channel(16);
        let ctx = make_ctx(Arc::clone(&registry));

        // Spawn the dispatch loop.
        let handle = tokio::spawn(run_cluster_dispatch_loop(ctx, rx));

        // Register a completion entry matching the payload we will send.
        let (oneshot_tx, oneshot_rx) = oneshot::channel();
        registry.insert("exec-1:peer-node".to_string(), oneshot_tx);

        // Serialize a DagComplete and inject via handle_cluster_peer_frame.
        let payload = DagCompletePayload {
            execution_id: "exec-1".to_string(),
            node_id: "peer-node".to_string(),
            success: true,
            error: None,
            results: Some(vec![1, 2, 3]),
        };
        let msg = ClusterMessage::DagComplete(payload.clone());
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");

        handle_cluster_peer_frame(&bytes, "peer-node".to_string(), &tx)
            .expect("frame should be accepted");

        // The oneshot should resolve with the payload.
        let result = tokio::time::timeout(Duration::from_secs(2), oneshot_rx)
            .await
            .expect("should not timeout")
            .expect("oneshot should resolve");

        assert_eq!(result.execution_id, "exec-1");
        assert_eq!(result.node_id, "peer-node");
        assert!(result.success);
        assert_eq!(result.results, Some(vec![1, 2, 3]));

        // The registry entry should have been removed.
        assert!(!registry.contains_key("exec-1:peer-node"));

        // Drop sender to close the loop.
        drop(tx);
        let _ = handle.await;
    }

    // -- AC4: Malformed bytes log warning and do not crash the dispatch loop --

    #[test]
    fn malformed_bytes_return_deserialize_error() {
        let (tx, _rx) = mpsc::channel(16);
        let result = handle_cluster_peer_frame(b"garbage", "bad-node".to_string(), &tx);

        assert!(result.is_err());
        assert!(
            matches!(result, Err(HandleFrameError::Deserialize(_))),
            "expected Deserialize error"
        );
    }

    // -- AC5: Non-DAG variants pass through without error --

    #[tokio::test]
    async fn non_dag_variant_does_not_crash_loop() {
        let registry: Arc<DashMap<String, oneshot::Sender<DagCompletePayload>>> =
            Arc::new(DashMap::new());

        let (tx, rx) = mpsc::channel(16);
        let ctx = make_ctx(Arc::clone(&registry));

        let handle = tokio::spawn(run_cluster_dispatch_loop(ctx, rx));

        // Send a Heartbeat (non-DAG variant).
        let heartbeat = ClusterMessage::Heartbeat(HeartbeatPayload {
            sender_id: "node-a".to_string(),
            timestamp_ms: 12345,
            members_view_version: 1,
            suspected_nodes: Vec::new(),
        });
        let bytes = rmp_serde::to_vec_named(&heartbeat).expect("serialize");
        handle_cluster_peer_frame(&bytes, "node-a".to_string(), &tx)
            .expect("heartbeat frame accepted");

        // Now send a DagComplete to verify the loop is still running.
        let (oneshot_tx, oneshot_rx) = oneshot::channel();
        registry.insert("exec-probe:prober".to_string(), oneshot_tx);

        let probe_payload = DagCompletePayload {
            execution_id: "exec-probe".to_string(),
            node_id: "prober".to_string(),
            success: true,
            error: None,
            results: None,
        };
        let probe_msg = ClusterMessage::DagComplete(probe_payload);
        let probe_bytes = rmp_serde::to_vec_named(&probe_msg).expect("serialize");
        handle_cluster_peer_frame(&probe_bytes, "prober".to_string(), &tx)
            .expect("probe frame accepted");

        // The oneshot should resolve, proving the loop survived the non-DAG variant.
        let result = tokio::time::timeout(Duration::from_secs(2), oneshot_rx)
            .await
            .expect("should not timeout")
            .expect("oneshot should resolve");

        assert_eq!(result.execution_id, "exec-probe");

        drop(tx);
        let _ = handle.await;
    }

    // -- Channel closed error --

    #[test]
    fn channel_closed_returns_error() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx); // Close the receiver so try_send fails.

        let msg = ClusterMessage::Heartbeat(HeartbeatPayload {
            sender_id: "node-x".to_string(),
            timestamp_ms: 0,
            members_view_version: 0,
            suspected_nodes: Vec::new(),
        });
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");

        let result = handle_cluster_peer_frame(&bytes, "node-x".to_string(), &tx);
        assert!(matches!(result, Err(HandleFrameError::ChannelClosed)));
    }
}
