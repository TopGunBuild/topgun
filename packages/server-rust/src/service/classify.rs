//! Message classification: converts `Message` into typed `Operation` variants.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use topgun_core::messages::Message;
use topgun_core::{hash_to_partition, HLC};

use super::config::ServerConfig;
use super::operation::{
    service_names, CallerOrigin, ClassifyError, Operation, OperationContext,
};

// ---------------------------------------------------------------------------
// OperationService
// ---------------------------------------------------------------------------

/// Classifies incoming `Message` values into typed `Operation` variants.
///
/// Each call generates a fresh HLC timestamp and a unique call ID. The
/// `partition_id` is computed from relevant key fields when available.
pub struct OperationService {
    hlc: Arc<parking_lot::Mutex<HLC>>,
    config: Arc<ServerConfig>,
    call_id_counter: AtomicU64,
}

impl OperationService {
    /// Create a new `OperationService`.
    #[must_use]
    pub fn new(hlc: Arc<parking_lot::Mutex<HLC>>, config: Arc<ServerConfig>) -> Self {
        Self {
            hlc,
            config,
            call_id_counter: AtomicU64::new(1),
        }
    }

    /// Generate a unique call ID for each operation.
    fn next_call_id(&self) -> u64 {
        self.call_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Generate a fresh HLC timestamp.
    fn now(&self) -> topgun_core::Timestamp {
        self.hlc.lock().now()
    }

    /// Build an `OperationContext` for the given service and optional partition key.
    fn make_ctx(
        &self,
        service_name: &'static str,
        client_id: Option<String>,
        caller_origin: CallerOrigin,
        partition_key: Option<&str>,
    ) -> OperationContext {
        let mut ctx = OperationContext::new(
            self.next_call_id(),
            service_name,
            self.now(),
            self.config.default_operation_timeout_ms,
        );
        ctx.caller_origin = caller_origin;
        ctx.client_id = client_id;
        ctx.partition_id = partition_key.map(hash_to_partition);
        ctx
    }

    /// Classify a `Message` into an `Operation`.
    ///
    /// Client-to-server messages produce `Ok(Operation)`.
    /// Server-to-client responses, transport envelopes, auth messages, and
    /// cluster-internal messages produce `Err(ClassifyError)`.
    ///
    /// # Errors
    ///
    /// Returns `ClassifyError` for messages that are not classifiable as operations:
    /// - `ClassifyError::ServerToClient` for response messages and cluster-internal messages
    /// - `ClassifyError::TransportEnvelope` for `Batch` messages
    /// - `ClassifyError::AuthMessage` for `Auth` and `AuthRequired` messages
    #[allow(clippy::too_many_lines)]
    pub fn classify(
        &self,
        msg: Message,
        client_id: Option<String>,
        caller_origin: CallerOrigin,
    ) -> Result<Operation, ClassifyError> {
        match msg {
            // ----- CRDT domain (service_name = "crdt") -----

            Message::ClientOp(payload) => {
                let partition_key = Some(payload.payload.key.as_str());
                let ctx = self.make_ctx(
                    service_names::CRDT,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::ClientOp { ctx, payload })
            }
            Message::OpBatch(payload) => {
                let ctx =
                    self.make_ctx(service_names::CRDT, client_id, caller_origin, None);
                Ok(Operation::OpBatch { ctx, payload })
            }

            // ----- Sync domain (service_name = "sync") -----

            Message::SyncInit(payload) => {
                let partition_key = Some(payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::SYNC,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::SyncInit { ctx, payload })
            }
            Message::MerkleReqBucket(payload) => {
                let partition_key = Some(payload.payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::SYNC,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::MerkleReqBucket { ctx, payload })
            }
            Message::ORMapSyncInit(payload) => {
                let partition_key = Some(payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::SYNC,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::ORMapSyncInit { ctx, payload })
            }
            Message::ORMapMerkleReqBucket(payload) => {
                let partition_key = Some(payload.payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::SYNC,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::ORMapMerkleReqBucket { ctx, payload })
            }
            Message::ORMapDiffRequest(payload) => {
                let partition_key = Some(payload.payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::SYNC,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::ORMapDiffRequest { ctx, payload })
            }
            Message::ORMapPushDiff(payload) => {
                let partition_key = Some(payload.payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::SYNC,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::ORMapPushDiff { ctx, payload })
            }

            // ----- Query domain (service_name = "query") -----

            Message::QuerySub(payload) => {
                let ctx = self.make_ctx(
                    service_names::QUERY,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::QuerySubscribe { ctx, payload })
            }
            Message::QueryUnsub(payload) => {
                let ctx = self.make_ctx(
                    service_names::QUERY,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::QueryUnsubscribe { ctx, payload })
            }

            // ----- Messaging domain (service_name = "messaging") -----

            Message::TopicSub { payload } => {
                let partition_key = Some(payload.topic.as_str());
                let ctx = self.make_ctx(
                    service_names::MESSAGING,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::TopicSubscribe { ctx, payload })
            }
            Message::TopicUnsub { payload } => {
                let partition_key = Some(payload.topic.as_str());
                let ctx = self.make_ctx(
                    service_names::MESSAGING,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::TopicUnsubscribe { ctx, payload })
            }
            Message::TopicPub { payload } => {
                let partition_key = Some(payload.topic.as_str());
                let ctx = self.make_ctx(
                    service_names::MESSAGING,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::TopicPublish { ctx, payload })
            }

            // ----- Coordination domain (service_name = "coordination") -----

            Message::LockRequest { payload } => {
                let partition_key = Some(payload.name.as_str());
                let ctx = self.make_ctx(
                    service_names::COORDINATION,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::LockRequest { ctx, payload })
            }
            Message::LockRelease { payload } => {
                let partition_key = Some(payload.name.as_str());
                let ctx = self.make_ctx(
                    service_names::COORDINATION,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::LockRelease { ctx, payload })
            }
            Message::PartitionMapRequest { payload } => {
                let ctx = self.make_ctx(
                    service_names::COORDINATION,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::PartitionMapRequest { ctx, payload })
            }
            Message::Ping(payload) => {
                let ctx = self.make_ctx(
                    service_names::COORDINATION,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::Ping { ctx, payload })
            }

            // ----- Search domain (service_name = "search") -----

            Message::Search { payload } => {
                let ctx = self.make_ctx(
                    service_names::SEARCH,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::Search { ctx, payload })
            }
            Message::SearchSub { payload } => {
                let ctx = self.make_ctx(
                    service_names::SEARCH,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::SearchSubscribe { ctx, payload })
            }
            Message::SearchUnsub { payload } => {
                let ctx = self.make_ctx(
                    service_names::SEARCH,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::SearchUnsubscribe { ctx, payload })
            }

            // ----- Persistence domain (service_name = "persistence") -----

            Message::CounterRequest { payload } => {
                let partition_key = Some(payload.name.as_str());
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::CounterRequest { ctx, payload })
            }
            Message::CounterSync { payload } => {
                let partition_key = Some(payload.name.as_str());
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::CounterSync { ctx, payload })
            }
            Message::EntryProcess(payload) => {
                let partition_key = Some(payload.key.as_str());
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::EntryProcess { ctx, payload })
            }
            Message::EntryProcessBatch(payload) => {
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    None,
                );
                Ok(Operation::EntryProcessBatch { ctx, payload })
            }
            Message::RegisterResolver(payload) => {
                let partition_key = Some(payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::RegisterResolver { ctx, payload })
            }
            Message::UnregisterResolver(payload) => {
                let partition_key = Some(payload.map_name.as_str());
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::UnregisterResolver { ctx, payload })
            }
            Message::ListResolvers(payload) => {
                let partition_key = payload.map_name.as_deref();
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::ListResolvers { ctx, payload })
            }
            Message::JournalSubscribe(payload) => {
                let partition_key = payload.map_name.as_deref();
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::JournalSubscribe { ctx, payload })
            }
            Message::JournalUnsubscribe(payload) => {
                let partition_key: Option<&str> = None;
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::JournalUnsubscribe { ctx, payload })
            }
            Message::JournalRead(payload) => {
                let partition_key = payload.map_name.as_deref();
                let ctx = self.make_ctx(
                    service_names::PERSISTENCE,
                    client_id,
                    caller_origin,
                    partition_key,
                );
                Ok(Operation::JournalRead { ctx, payload })
            }

            // ----- Server-to-client responses -> ClassifyError::ServerToClient -----

            Message::OpAck(_) => Err(ClassifyError::ServerToClient { variant: "OpAck" }),
            Message::OpRejected(_) => {
                Err(ClassifyError::ServerToClient { variant: "OpRejected" })
            }
            Message::SyncRespRoot(_) => {
                Err(ClassifyError::ServerToClient { variant: "SyncRespRoot" })
            }
            Message::SyncRespBuckets(_) => {
                Err(ClassifyError::ServerToClient { variant: "SyncRespBuckets" })
            }
            Message::SyncRespLeaf(_) => {
                Err(ClassifyError::ServerToClient { variant: "SyncRespLeaf" })
            }
            Message::ORMapSyncRespRoot(_) => {
                Err(ClassifyError::ServerToClient { variant: "ORMapSyncRespRoot" })
            }
            Message::ORMapSyncRespBuckets(_) => Err(ClassifyError::ServerToClient {
                variant: "ORMapSyncRespBuckets",
            }),
            Message::ORMapSyncRespLeaf(_) => {
                Err(ClassifyError::ServerToClient { variant: "ORMapSyncRespLeaf" })
            }
            Message::ORMapDiffResponse(_) => {
                Err(ClassifyError::ServerToClient { variant: "ORMapDiffResponse" })
            }
            Message::QueryResp(_) => {
                Err(ClassifyError::ServerToClient { variant: "QueryResp" })
            }
            Message::QueryUpdate { .. } => {
                Err(ClassifyError::ServerToClient { variant: "QueryUpdate" })
            }
            Message::SearchResp { .. } => {
                Err(ClassifyError::ServerToClient { variant: "SearchResp" })
            }
            Message::SearchUpdate { .. } => {
                Err(ClassifyError::ServerToClient { variant: "SearchUpdate" })
            }
            Message::CounterResponse { .. } => {
                Err(ClassifyError::ServerToClient { variant: "CounterResponse" })
            }
            Message::CounterUpdate { .. } => {
                Err(ClassifyError::ServerToClient { variant: "CounterUpdate" })
            }
            Message::EntryProcessResponse(_) => Err(ClassifyError::ServerToClient {
                variant: "EntryProcessResponse",
            }),
            Message::EntryProcessBatchResponse(_) => Err(ClassifyError::ServerToClient {
                variant: "EntryProcessBatchResponse",
            }),
            Message::JournalEvent { .. } => {
                Err(ClassifyError::ServerToClient { variant: "JournalEvent" })
            }
            Message::JournalReadResponse(_) => {
                Err(ClassifyError::ServerToClient { variant: "JournalReadResponse" })
            }
            Message::RegisterResolverResponse(_) => Err(ClassifyError::ServerToClient {
                variant: "RegisterResolverResponse",
            }),
            Message::UnregisterResolverResponse(_) => Err(ClassifyError::ServerToClient {
                variant: "UnregisterResolverResponse",
            }),
            Message::ListResolversResponse(_) => Err(ClassifyError::ServerToClient {
                variant: "ListResolversResponse",
            }),
            Message::MergeRejected(_) => {
                Err(ClassifyError::ServerToClient { variant: "MergeRejected" })
            }
            Message::ServerEvent { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ServerEvent" })
            }
            Message::ServerBatchEvent { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ServerBatchEvent" })
            }
            Message::GcPrune { .. } => {
                Err(ClassifyError::ServerToClient { variant: "GcPrune" })
            }
            Message::AuthAck(_) => {
                Err(ClassifyError::ServerToClient { variant: "AuthAck" })
            }
            Message::AuthFail(_) => {
                Err(ClassifyError::ServerToClient { variant: "AuthFail" })
            }
            Message::Error { .. } => {
                Err(ClassifyError::ServerToClient { variant: "Error" })
            }
            Message::LockGranted { .. } => {
                Err(ClassifyError::ServerToClient { variant: "LockGranted" })
            }
            Message::LockReleased { .. } => {
                Err(ClassifyError::ServerToClient { variant: "LockReleased" })
            }
            Message::SyncResetRequired { .. } => {
                Err(ClassifyError::ServerToClient { variant: "SyncResetRequired" })
            }
            Message::Pong(_) => {
                Err(ClassifyError::ServerToClient { variant: "Pong" })
            }
            Message::PartitionMap { .. } => {
                Err(ClassifyError::ServerToClient { variant: "PartitionMap" })
            }
            Message::TopicMessage { .. } => {
                Err(ClassifyError::ServerToClient { variant: "TopicMessage" })
            }

            // ----- Cluster-internal messages -> ClassifyError::ServerToClient -----
            // These are node-to-node, not client-to-server. They will be handled
            // by a separate cluster message path in Phase 3+.

            Message::ClusterSubRegister { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSubRegister" })
            }
            Message::ClusterSubAck { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSubAck" })
            }
            Message::ClusterSubUpdate { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSubUpdate" })
            }
            Message::ClusterSubUnregister { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSubUnregister" })
            }
            Message::ClusterSearchReq { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSearchReq" })
            }
            Message::ClusterSearchResp { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSearchResp" })
            }
            Message::ClusterSearchSubscribe { .. } => Err(ClassifyError::ServerToClient {
                variant: "ClusterSearchSubscribe",
            }),
            Message::ClusterSearchUnsubscribe { .. } => Err(ClassifyError::ServerToClient {
                variant: "ClusterSearchUnsubscribe",
            }),
            Message::ClusterSearchUpdate { .. } => {
                Err(ClassifyError::ServerToClient { variant: "ClusterSearchUpdate" })
            }

            // ----- Transport envelope -> ClassifyError::TransportEnvelope -----

            Message::Batch(_) => {
                Err(ClassifyError::TransportEnvelope { variant: "Batch" })
            }

            // ----- Auth messages -> ClassifyError::AuthMessage -----

            Message::Auth(_) => Err(ClassifyError::AuthMessage { variant: "Auth" }),
            Message::AuthRequired(_) => {
                Err(ClassifyError::AuthMessage { variant: "AuthRequired" })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::{SystemClock, HLC};

    use super::*;
    use crate::service::operation::service_names;

    fn make_service() -> OperationService {
        let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            "test-node".to_string(),
            Box::new(SystemClock),
        )));
        let config = Arc::new(ServerConfig {
            node_id: "test-node".to_string(),
            default_operation_timeout_ms: 5000,
            ..ServerConfig::default()
        });
        OperationService::new(hlc, config)
    }

    #[test]
    fn classify_client_op() {
        let svc = make_service();
        let msg = Message::ClientOp(topgun_core::messages::sync::ClientOpMessage {
            payload: topgun_core::ClientOp {
                id: None,
                map_name: "users".to_string(),
                key: "alice".to_string(),
                op_type: None,
                record: None,
                or_record: None,
                or_tag: None,
                write_concern: None,
                timeout: None,
            },
        });
        let op = svc
            .classify(msg, Some("client-1".to_string()), CallerOrigin::Client)
            .unwrap();
        assert_eq!(op.ctx().service_name, service_names::CRDT);
        assert!(op.ctx().partition_id.is_some());
        assert_eq!(op.ctx().client_id.as_deref(), Some("client-1"));
    }

    #[test]
    fn classify_ping_routes_to_coordination() {
        let svc = make_service();
        let msg = Message::Ping(topgun_core::messages::PingData {
            timestamp: 1_700_000_000_000,
        });
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::COORDINATION);
    }

    #[test]
    fn classify_topic_sub_routes_to_messaging() {
        let svc = make_service();
        let msg = Message::TopicSub {
            payload: topgun_core::messages::TopicSubPayload {
                topic: "chat.room1".to_string(),
            },
        };
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::MESSAGING);
        assert!(op.ctx().partition_id.is_some());
    }

    #[test]
    fn classify_search_routes_to_search() {
        let svc = make_service();
        let msg = Message::Search {
            payload: topgun_core::messages::SearchPayload {
                request_id: "req-1".to_string(),
                map_name: "products".to_string(),
                query: "widget".to_string(),
                options: None,
            },
        };
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::SEARCH);
    }

    #[test]
    fn classify_counter_request_routes_to_persistence() {
        let svc = make_service();
        let msg = Message::CounterRequest {
            payload: topgun_core::messages::CounterRequestPayload {
                name: "page-views".to_string(),
            },
        };
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::PERSISTENCE);
        assert!(op.ctx().partition_id.is_some());
    }

    #[test]
    fn classify_lock_request_routes_to_coordination() {
        let svc = make_service();
        let msg = Message::LockRequest {
            payload: topgun_core::messages::LockRequestPayload {
                request_id: "req-1".to_string(),
                name: "my-lock".to_string(),
                ttl: Some(5000),
            },
        };
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::COORDINATION);
        assert!(op.ctx().partition_id.is_some());
    }

    #[test]
    fn classify_server_to_client_returns_error() {
        let svc = make_service();
        let msg = Message::Pong(topgun_core::messages::PongData {
            timestamp: 1_700_000_000_000,
            server_time: 1_700_000_000_001,
        });
        let err = svc.classify(msg, None, CallerOrigin::Client).unwrap_err();
        assert!(matches!(err, ClassifyError::ServerToClient { variant: "Pong" }));
    }

    #[test]
    fn classify_batch_returns_transport_envelope_error() {
        let svc = make_service();
        let msg = Message::Batch(topgun_core::messages::sync::BatchMessage {
            count: 0,
            data: vec![],
        });
        let err = svc.classify(msg, None, CallerOrigin::Client).unwrap_err();
        assert!(matches!(
            err,
            ClassifyError::TransportEnvelope { variant: "Batch" }
        ));
    }

    #[test]
    fn classify_auth_returns_auth_error() {
        let svc = make_service();
        let msg = Message::Auth(topgun_core::messages::AuthMessage {
            token: "test-token".to_string(),
            protocol_version: None,
        });
        let err = svc.classify(msg, None, CallerOrigin::Client).unwrap_err();
        assert!(matches!(
            err,
            ClassifyError::AuthMessage { variant: "Auth" }
        ));
    }

    #[test]
    fn classify_cluster_internal_returns_server_to_client_error() {
        let svc = make_service();
        let msg = Message::ClusterSubAck {
            payload: topgun_core::messages::ClusterSubAckPayload {
                subscription_id: "sub-1".to_string(),
                node_id: "node-1".to_string(),
                success: true,
                error: None,
                initial_results: None,
                total_hits: None,
            },
        };
        let err = svc.classify(msg, None, CallerOrigin::Client).unwrap_err();
        assert!(matches!(
            err,
            ClassifyError::ServerToClient { variant: "ClusterSubAck" }
        ));
    }

    #[test]
    fn call_ids_are_unique_and_monotonic() {
        let svc = make_service();
        let msg1 = Message::Ping(topgun_core::messages::PingData {
            timestamp: 1_700_000_000_000,
        });
        let msg2 = Message::Ping(topgun_core::messages::PingData {
            timestamp: 1_700_000_000_001,
        });
        let op1 = svc.classify(msg1, None, CallerOrigin::Client).unwrap();
        let op2 = svc.classify(msg2, None, CallerOrigin::Client).unwrap();
        assert!(op2.ctx().call_id > op1.ctx().call_id);
    }

    #[test]
    fn classify_partition_map_request_routes_to_coordination() {
        let svc = make_service();
        let msg = Message::PartitionMapRequest { payload: None };
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::COORDINATION);
        assert!(op.ctx().partition_id.is_none());
    }

    #[test]
    fn classify_journal_subscribe_routes_to_persistence() {
        let svc = make_service();
        let msg = Message::JournalSubscribe(topgun_core::messages::JournalSubscribeData {
            request_id: "req-1".to_string(),
            from_sequence: None,
            map_name: Some("events".to_string()),
            types: None,
        });
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::PERSISTENCE);
        assert!(op.ctx().partition_id.is_some());
    }

    #[test]
    fn classify_entry_process_routes_to_persistence() {
        let svc = make_service();
        let msg = Message::EntryProcess(topgun_core::messages::EntryProcessData {
            request_id: "req-1".to_string(),
            map_name: "users".to_string(),
            key: "alice".to_string(),
            processor: topgun_core::messages::EntryProcessor {
                name: "test-processor".to_string(),
                code: "return entry;".to_string(),
                args: None,
            },
        });
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::PERSISTENCE);
        assert!(op.ctx().partition_id.is_some());
    }

    #[test]
    fn classify_query_sub_routes_to_query() {
        let svc = make_service();
        let msg = Message::QuerySub(topgun_core::messages::QuerySubMessage {
            payload: topgun_core::messages::QuerySubPayload {
                query_id: "sub-1".to_string(),
                map_name: "users".to_string(),
                query: topgun_core::messages::base::Query::default(),
            },
        });
        let op = svc.classify(msg, None, CallerOrigin::Client).unwrap();
        assert_eq!(op.ctx().service_name, service_names::QUERY);
    }
}
