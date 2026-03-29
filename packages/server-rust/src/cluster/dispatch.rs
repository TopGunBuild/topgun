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
use crate::service::domain::dag_dispatch::{handle_dag_complete, handle_dag_data, handle_dag_execute};
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
    let message: ClusterMessage =
        rmp_serde::from_slice(bytes).map_err(HandleFrameError::Deserialize)?;

    let inbound = InboundClusterMessage {
        sender_node_id,
        message,
    };

    tx.try_send(inbound).map_err(|_| HandleFrameError::ChannelClosed)
}
