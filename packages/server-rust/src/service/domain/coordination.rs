//! Coordination domain service handling Ping, `PartitionMapRequest`, and lock operations.
//!
//! This is the first real domain service replacing a `domain_stub!` macro.
//! It validates the full message pipeline: classify -> route -> handle -> respond.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use tower::Service;

use topgun_core::messages::{self, Message};

use tracing::Instrument;

use crate::cluster::state::ClusterState;
use crate::network::connection::ConnectionRegistry;
use crate::service::operation::{service_names, Operation, OperationError, OperationResponse};
use crate::service::registry::{ManagedService, ServiceContext};

// ---------------------------------------------------------------------------
// CoordinationService
// ---------------------------------------------------------------------------

/// Real coordination domain service handling Ping, `PartitionMapRequest`,
/// and lock operations. Replaces the `domain_stub!(CoordinationService, ...)`
/// macro-generated stub.
pub struct CoordinationService {
    cluster_state: Arc<ClusterState>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl CoordinationService {
    /// Creates a new `CoordinationService` with its required dependencies.
    #[must_use]
    pub fn new(
        cluster_state: Arc<ClusterState>,
        connection_registry: Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            cluster_state,
            connection_registry,
        }
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for CoordinationService {
    fn name(&self) -> &'static str {
        service_names::COORDINATION
    }

    async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn reset(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// tower::Service<Operation> implementation
// ---------------------------------------------------------------------------

impl Service<Operation> for Arc<CoordinationService> {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let svc = Arc::clone(self);
        let service_name = op.ctx().service_name;
        let call_id = op.ctx().call_id;
        let caller_origin = format!("{:?}", op.ctx().caller_origin);

        let span = tracing::info_span!(
            "domain_op",
            service = service_name,
            call_id = call_id,
            caller_origin = %caller_origin,
        );

        Box::pin(
            async move {
                match op {
                    Operation::Ping { ctx, payload } => svc.handle_ping(&ctx, payload).await,
                    Operation::PartitionMapRequest { payload, .. } => {
                        Ok(svc.handle_partition_map_request(payload.as_ref()))
                    }
                    Operation::LockRequest { ctx, .. } | Operation::LockRelease { ctx, .. } => {
                        Ok(OperationResponse::NotImplemented {
                            service_name: service_names::COORDINATION,
                            call_id: ctx.call_id,
                        })
                    }
                    _ => Err(OperationError::WrongService),
                }
            }
            .instrument(span),
        )
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

impl CoordinationService {
    /// Handles a Ping operation: echoes the client timestamp, records server
    /// time, and updates the caller's heartbeat metadata if a connection is
    /// identified.
    async fn handle_ping(
        &self,
        ctx: &crate::service::operation::OperationContext,
        payload: messages::PingData,
    ) -> Result<OperationResponse, OperationError> {
        #[allow(clippy::cast_possible_truncation)]
        let server_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Update heartbeat for the caller's connection if known.
        if let Some(conn_id) = ctx.connection_id {
            if let Some(handle) = self.connection_registry.get(conn_id) {
                let mut meta = handle.metadata.write().await;
                meta.last_heartbeat = Instant::now();
            }
        }

        Ok(OperationResponse::Message(Box::new(Message::Pong(
            messages::PongData {
                timestamp: payload.timestamp,
                server_time,
            },
        ))))
    }

    /// Handles a `PartitionMapRequest`: returns the current partition map if
    /// the client's version is stale, or Empty if already up-to-date.
    fn handle_partition_map_request(
        &self,
        payload: Option<&messages::PartitionMapRequestPayload>,
    ) -> OperationResponse {
        let members_view = self.cluster_state.current_view();
        let table_version = self.cluster_state.partition_table.version();

        // Widen u32 -> u64 safely; missing payload treated as version 0.
        let client_version = payload.and_then(|p| p.current_version).map_or(0, u64::from);

        if client_version < table_version {
            let map = self
                .cluster_state
                .partition_table
                .to_partition_map(&members_view);
            OperationResponse::Message(Box::new(Message::PartitionMap { payload: map }))
        } else {
            OperationResponse::Empty
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::SystemTime;

    use topgun_core::messages::{self, Message};
    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::cluster::state::ClusterState;
    use crate::cluster::types::{ClusterConfig, MemberInfo, MembersView, NodeState};
    use crate::network::config::ConnectionConfig;
    use crate::network::connection::{ConnectionKind, ConnectionRegistry};
    use crate::service::operation::{service_names, OperationContext, OperationResponse};

    /// Helper: build a test `CoordinationService` with default cluster state.
    fn make_service() -> (Arc<CoordinationService>, Arc<ClusterState>) {
        let config = Arc::new(ClusterConfig::default());
        let (state, _rx) = ClusterState::new(config, "test-node".to_string());
        let state = Arc::new(state);
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(CoordinationService::new(Arc::clone(&state), registry));
        (svc, state)
    }

    /// Helper: build a test `CoordinationService` with a connection registry.
    fn make_service_with_registry() -> (
        Arc<CoordinationService>,
        Arc<ClusterState>,
        Arc<ConnectionRegistry>,
    ) {
        let config = Arc::new(ClusterConfig::default());
        let (state, _rx) = ClusterState::new(config, "test-node".to_string());
        let state = Arc::new(state);
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(CoordinationService::new(
            Arc::clone(&state),
            Arc::clone(&registry),
        ));
        (svc, state, registry)
    }

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "test-node".to_string(),
        }
    }

    fn make_ctx(service_name: &'static str) -> OperationContext {
        OperationContext::new(1, service_name, make_timestamp(), 5000)
    }

    fn make_member(node_id: &str) -> MemberInfo {
        MemberInfo {
            node_id: node_id.to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state: NodeState::Active,
            join_version: 1,
        }
    }

    // -- AC1: Ping -> Pong --

    #[tokio::test]
    async fn ping_returns_pong_with_echoed_timestamp() {
        let (svc, _state) = make_service();
        let op = Operation::Ping {
            ctx: make_ctx(service_names::COORDINATION),
            payload: messages::PingData { timestamp: 42 },
        };

        let resp = svc.oneshot(op).await.unwrap();

        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::Pong(pong) => {
                    assert_eq!(pong.timestamp, 42);
                    // server_time should be within 1 second of now.
                    #[allow(clippy::cast_possible_truncation)]
                    let now_ms = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    assert!(
                        now_ms.abs_diff(pong.server_time) < 1000,
                        "server_time {} should be within 1s of now {}",
                        pong.server_time,
                        now_ms
                    );
                }
                other => panic!("expected Pong, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        }
    }

    // -- AC2: PartitionMapRequest (stale version) --

    #[tokio::test]
    async fn partition_map_request_stale_version_returns_map() {
        let (svc, state) = make_service();

        // Set up partition table at version 3.
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec![]);
        let _ = state.partition_table.increment_version(); // v1
        let _ = state.partition_table.increment_version(); // v2
        let _ = state.partition_table.increment_version(); // v3
        assert_eq!(state.partition_table.version(), 3);

        // Add a member so the partition map has nodes.
        state.update_view(MembersView {
            version: 1,
            members: vec![make_member("node-1")],
        });

        let op = Operation::PartitionMapRequest {
            ctx: make_ctx(service_names::COORDINATION),
            payload: Some(messages::PartitionMapRequestPayload {
                current_version: Some(1),
            }),
        };

        let resp = svc.oneshot(op).await.unwrap();

        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::PartitionMap { payload } => {
                    assert_eq!(payload.version, 3);
                }
                other => panic!("expected PartitionMap, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        }
    }

    // -- AC3: PartitionMapRequest (current version) --

    #[tokio::test]
    async fn partition_map_request_current_version_returns_empty() {
        let (svc, state) = make_service();

        // Set table to version 3.
        let _ = state.partition_table.increment_version();
        let _ = state.partition_table.increment_version();
        let _ = state.partition_table.increment_version();

        let op = Operation::PartitionMapRequest {
            ctx: make_ctx(service_names::COORDINATION),
            payload: Some(messages::PartitionMapRequestPayload {
                current_version: Some(3),
            }),
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(
            matches!(resp, OperationResponse::Empty),
            "expected Empty, got {resp:?}"
        );
    }

    // -- AC4: PartitionMapRequest (no payload, version > 0) --

    #[tokio::test]
    async fn partition_map_request_no_payload_returns_map_when_version_gt_zero() {
        let (svc, state) = make_service();

        // Set table to version 1.
        state
            .partition_table
            .set_owner(0, "node-1".to_string(), vec![]);
        let _ = state.partition_table.increment_version();
        assert_eq!(state.partition_table.version(), 1);

        state.update_view(MembersView {
            version: 1,
            members: vec![make_member("node-1")],
        });

        let op = Operation::PartitionMapRequest {
            ctx: make_ctx(service_names::COORDINATION),
            payload: None,
        };

        let resp = svc.oneshot(op).await.unwrap();

        match resp {
            OperationResponse::Message(msg) => match *msg {
                Message::PartitionMap { payload } => {
                    assert!(payload.version > 0);
                }
                other => panic!("expected PartitionMap, got {other:?}"),
            },
            other => panic!("expected Message, got {other:?}"),
        }
    }

    // -- AC5: Heartbeat side-effect --

    #[tokio::test]
    async fn ping_updates_connection_heartbeat() {
        let (svc, _state, registry) = make_service_with_registry();

        let config = ConnectionConfig::default();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        // Record the initial heartbeat time.
        let initial_heartbeat = {
            let meta = handle.metadata.read().await;
            meta.last_heartbeat
        };

        // Wait a small amount so the new heartbeat differs.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut ctx = make_ctx(service_names::COORDINATION);
        ctx.connection_id = Some(conn_id);

        let op = Operation::Ping {
            ctx,
            payload: messages::PingData { timestamp: 100 },
        };

        let _resp = svc.oneshot(op).await.unwrap();

        let updated_heartbeat = {
            let meta = handle.metadata.read().await;
            meta.last_heartbeat
        };

        assert!(
            updated_heartbeat > initial_heartbeat,
            "heartbeat should have been updated"
        );
    }

    // -- AC6: Lock operations return NotImplemented --

    #[tokio::test]
    async fn lock_request_returns_not_implemented() {
        let (svc, _state) = make_service();
        let op = Operation::LockRequest {
            ctx: make_ctx(service_names::COORDINATION),
            payload: messages::LockRequestPayload {
                request_id: "req-1".to_string(),
                name: "test-lock".to_string(),
                ttl: None,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "coordination",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn lock_release_returns_not_implemented() {
        let (svc, _state) = make_service();
        let op = Operation::LockRelease {
            ctx: make_ctx(service_names::COORDINATION),
            payload: messages::LockReleasePayload {
                request_id: None,
                name: "test-lock".to_string(),
                fencing_token: 0,
            },
        };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "coordination",
                ..
            }
        ));
    }

    // -- AC7: Wrong service rejection --

    #[tokio::test]
    async fn wrong_service_returns_error() {
        let (svc, _state) = make_service();
        let op = Operation::GarbageCollect {
            ctx: make_ctx(service_names::COORDINATION),
        };

        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::WrongService)),
            "expected WrongService, got {result:?}"
        );
    }

    // -- AC8: ManagedService lifecycle (name check) --

    #[test]
    fn managed_service_name() {
        let config = Arc::new(ClusterConfig::default());
        let (state, _rx) = ClusterState::new(config, "test-node".to_string());
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = CoordinationService::new(Arc::new(state), registry);
        assert_eq!(svc.name(), "coordination");
    }
}
