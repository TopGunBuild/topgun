//! Cluster message dispatch handlers for DAG execution.
//!
//! Handles inbound `DagExecute`, `DagComplete`, and `DagData` cluster messages
//! from peer nodes during distributed GROUP BY query execution.

use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use tokio::sync::oneshot;

use crate::cluster::messages::{ClusterMessage, DagCompletePayload, DagDataPayload, DagExecutePayload};
use crate::dag::executor::{DagExecutor, ExecutorContext};
use crate::dag::types::{Dag, ExecutionPlan};
use crate::network::connection::{ConnectionKind, ConnectionRegistry, OutboundMessage};
use crate::storage::RecordStoreFactory;

/// Handles an inbound `DagExecute` cluster message.
///
/// Deserializes the `ExecutionPlan`, builds a `Dag` from the descriptor,
/// executes locally for this node's assigned partitions, and sends a
/// `DagComplete` message back to the originating node via `ConnectionRegistry`.
///
/// # Errors
///
/// Returns an error if serialization of the `DagComplete` reply fails.
pub async fn handle_dag_execute(
    payload: DagExecutePayload,
    local_node_id: &str,
    record_store_factory: Arc<RecordStoreFactory>,
    sender_node_id: &str,
    connection_registry: Arc<ConnectionRegistry>,
) -> Result<()> {
    let execution_id = payload.execution_id.clone();

    let result = execute_local(
        &payload,
        local_node_id,
        Arc::clone(&record_store_factory),
    )
    .await;

    let complete_payload = match result {
        Ok(result_bytes) => DagCompletePayload {
            execution_id: execution_id.clone(),
            node_id: local_node_id.to_string(),
            success: true,
            error: None,
            results: Some(result_bytes),
        },
        Err(e) => DagCompletePayload {
            execution_id: execution_id.clone(),
            node_id: local_node_id.to_string(),
            success: false,
            error: Some(e.to_string()),
            results: None,
        },
    };

    // Send DagComplete back to the originating node using the send_to_peer pattern
    // from ClusterQueryCoordinator: iterate connections, match ClusterPeer by peer_node_id,
    // serialize as rmp_serde::to_vec_named, send as OutboundMessage::Binary.
    if let Ok(bytes) = rmp_serde::to_vec_named(&ClusterMessage::DagComplete(complete_payload)) {
        for handle in connection_registry.connections() {
            if handle.kind != ConnectionKind::ClusterPeer {
                continue;
            }
            let meta = handle.metadata.read().await;
            if meta.peer_node_id.as_deref() == Some(sender_node_id) {
                drop(meta);
                let _ = handle.try_send(OutboundMessage::Binary(bytes));
                break;
            }
        }
    }

    Ok(())
}

/// Executes the DAG locally for this node's assigned partitions.
/// Returns MsgPack-serialized `Vec<rmpv::Value>` results.
async fn execute_local(
    payload: &DagExecutePayload,
    local_node_id: &str,
    record_store_factory: Arc<RecordStoreFactory>,
) -> Result<Vec<u8>> {
    let plan: ExecutionPlan = rmp_serde::from_slice(&payload.plan)?;

    let partition_ids = plan
        .partition_assignment
        .get(local_node_id)
        .cloned()
        .unwrap_or_default();

    let dag = Dag::from_descriptor(
        &plan.plan,
        &|vd| crate::dag::coordinator::make_supplier_from_descriptor(vd, Arc::clone(&record_store_factory)),
    )?;

    let ctx = ExecutorContext {
        node_id: local_node_id.to_string(),
        partition_ids,
        record_store_factory,
    };

    let results = DagExecutor::new(dag, ctx, plan.config.timeout_ms)
        .execute()
        .await?;

    let bytes = rmp_serde::to_vec_named(&results)?;
    Ok(bytes)
}

/// Handles an inbound `DagComplete` cluster message.
///
/// Resolves the matching oneshot sender in the `completion_registry` so that
/// `ClusterQueryCoordinator::execute_distributed()` can collect the result.
pub fn handle_dag_complete(
    payload: DagCompletePayload,
    completion_registry: &DashMap<String, oneshot::Sender<DagCompletePayload>>,
) {
    let key = format!("{}:{}", payload.execution_id, payload.node_id);

    if let Some((_, sender)) = completion_registry.remove(&key) {
        // Deliver payload to the waiting coordinator; ignore send errors
        // (coordinator may have timed out and dropped the receiver).
        let _ = sender.send(payload);
    } else {
        // Late arrival or duplicate — coordinator already timed out or cleaned up.
        tracing::warn!(
            key = %key,
            "DagComplete arrived but no waiting receiver found (late arrival or duplicate)"
        );
    }
}

/// Handles an inbound `DagData` cluster message.
///
/// Currently a no-op placeholder. The current coordinator uses a fan-out/collect
/// model where each node executes independently and sends `DagComplete` on finish.
pub fn handle_dag_data(_payload: DagDataPayload) {
    // TODO: route to NetworkReceiverProcessor inbox when streaming DAG is implemented
    tracing::debug!("DagData received: dropped (streaming DAG not yet implemented)");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use dashmap::DashMap;
    use tokio::sync::oneshot;

    use crate::cluster::messages::DagCompletePayload;

    use super::handle_dag_complete;

    fn make_complete_payload(execution_id: &str, node_id: &str) -> DagCompletePayload {
        DagCompletePayload {
            execution_id: execution_id.to_string(),
            node_id: node_id.to_string(),
            success: true,
            error: None,
            results: None,
        }
    }

    #[test]
    fn handle_dag_complete_delivers_payload_to_waiting_receiver() {
        let registry: DashMap<String, oneshot::Sender<DagCompletePayload>> = DashMap::new();
        let (tx, mut rx) = oneshot::channel();
        registry.insert("exec-1:node-1".to_string(), tx);

        let payload = make_complete_payload("exec-1", "node-1");
        handle_dag_complete(payload, &registry);

        // Receiver should have the delivered payload
        let received = rx.try_recv().expect("payload should have been delivered");
        assert_eq!(received.execution_id, "exec-1");
        assert_eq!(received.node_id, "node-1");
        assert!(received.success);

        // Registry entry should have been removed after delivery
        assert!(!registry.contains_key("exec-1:node-1"));
    }

    #[test]
    fn handle_dag_complete_late_arrival_does_not_panic() {
        // When no receiver is registered, the handler should log a warning and
        // return without panicking (late arrival / coordinator already timed out).
        let registry: DashMap<String, oneshot::Sender<DagCompletePayload>> = DashMap::new();

        let payload = make_complete_payload("exec-99", "node-1");
        handle_dag_complete(payload, &registry);

        // Registry remains empty and no panic occurred
        assert!(registry.is_empty());
    }
}
