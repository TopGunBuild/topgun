//! `TopGun` Server — `WebSocket` server with clustering, partitioning, and `PostgreSQL` storage.

pub mod cluster;
pub mod network;
pub mod service;
pub mod storage;
pub mod traits;

pub use service::{
    CallerOrigin, ManagedService, Operation, OperationContext, OperationError, OperationResponse,
    OperationService, OperationRouter, ServerConfig, ServiceContext, ServiceRegistry,
};
pub use traits::{MapProvider, SchemaProvider, ServerStorage};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        // Empty body: if this test runs, the crate compiles and loads.
    }
}

/// Integration tests for the full operation pipeline.
///
/// Tests the end-to-end flow: Message -> classify -> pipeline -> router -> stub -> response.
#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;

    use topgun_core::messages::Message;
    use topgun_core::{SystemClock, HLC};
    use tower::{Service, ServiceExt};

    use crate::service::config::ServerConfig;
    use crate::service::domain::{
        CoordinationService, CrdtService, MessagingService, PersistenceService, QueryService,
        SearchService, SyncService,
    };
    use crate::service::middleware::build_operation_pipeline;
    use crate::service::operation::{service_names, CallerOrigin, OperationResponse};
    use crate::service::registry::{ServiceContext, ServiceRegistry};
    use crate::service::router::OperationRouter;
    use crate::service::{OperationService, ClassifyError};

    fn setup() -> (OperationService, OperationRouter, ServerConfig) {
        let config = ServerConfig {
            node_id: "integration-test-node".to_string(),
            default_operation_timeout_ms: 5000,
            max_concurrent_operations: 100,
            gc_interval_ms: 60_000,
            ..ServerConfig::default()
        };

        let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            config.node_id.clone(),
            Box::new(SystemClock),
        )));

        let classify_svc = OperationService::new(hlc, Arc::new(config.clone()));

        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, Arc::new(CrdtService));
        router.register(service_names::SYNC, Arc::new(SyncService));
        router.register(service_names::QUERY, Arc::new(QueryService));
        router.register(service_names::MESSAGING, Arc::new(MessagingService));
        router.register(service_names::COORDINATION, Arc::new(CoordinationService));
        router.register(service_names::SEARCH, Arc::new(SearchService));
        router.register(service_names::PERSISTENCE, Arc::new(PersistenceService));

        (classify_svc, router, config)
    }

    #[tokio::test]
    async fn full_pipeline_ping_to_not_implemented() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config);

        // Classify a Ping message.
        let msg = Message::Ping(topgun_core::messages::PingData {
            timestamp: 1_700_000_000_000,
        });
        let op = classify_svc
            .classify(msg, Some("client-1".to_string()), CallerOrigin::Client)
            .unwrap();

        assert_eq!(op.ctx().service_name, service_names::COORDINATION);

        // Route through the full pipeline.
        let resp = ServiceExt::ready(&mut pipeline)
            .await
            .unwrap()
            .call(op)
            .await
            .unwrap();

        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "coordination",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn full_pipeline_client_op_to_not_implemented() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config);

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
        let op = classify_svc
            .classify(msg, None, CallerOrigin::Client)
            .unwrap();

        assert_eq!(op.ctx().service_name, service_names::CRDT);
        assert!(op.ctx().partition_id.is_some());

        let resp = ServiceExt::ready(&mut pipeline)
            .await
            .unwrap()
            .call(op)
            .await
            .unwrap();

        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "crdt",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn full_pipeline_topic_subscribe_to_not_implemented() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config);

        let msg = Message::TopicSub {
            payload: topgun_core::messages::TopicSubPayload {
                topic: "chat.general".to_string(),
            },
        };
        let op = classify_svc
            .classify(msg, None, CallerOrigin::Client)
            .unwrap();

        assert_eq!(op.ctx().service_name, service_names::MESSAGING);

        let resp = ServiceExt::ready(&mut pipeline)
            .await
            .unwrap()
            .call(op)
            .await
            .unwrap();

        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "messaging",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn classify_rejects_server_to_client_message() {
        let (classify_svc, _, _) = setup();

        let msg = Message::Pong(topgun_core::messages::PongData {
            timestamp: 0,
            server_time: 0,
        });
        let err = classify_svc
            .classify(msg, None, CallerOrigin::Client)
            .unwrap_err();
        assert!(matches!(
            err,
            ClassifyError::ServerToClient { variant: "Pong" }
        ));
    }

    #[tokio::test]
    async fn service_registry_lifecycle() {
        let registry = ServiceRegistry::new();
        registry.register(CrdtService);
        registry.register(SyncService);
        registry.register(QueryService);
        registry.register(MessagingService);
        registry.register(CoordinationService);
        registry.register(SearchService);
        registry.register(PersistenceService);

        let config = ServerConfig::default();
        let ctx = ServiceContext {
            config: Arc::new(config),
        };

        // Init all services in order.
        registry.init_all(&ctx).await.unwrap();

        // Verify all accessible.
        assert!(registry.get::<CrdtService>().is_some());
        assert!(registry.get::<SyncService>().is_some());
        assert!(registry.get::<QueryService>().is_some());
        assert!(registry.get::<MessagingService>().is_some());
        assert!(registry.get::<CoordinationService>().is_some());
        assert!(registry.get::<SearchService>().is_some());
        assert!(registry.get::<PersistenceService>().is_some());

        // Shutdown all in reverse order.
        registry.shutdown_all(false).await.unwrap();
    }

    #[test]
    fn re_exports_accessible_from_crate_root() {
        // Verify that the key types are accessible from the crate root.
        let _config = crate::ServerConfig::default();
        let _registry = crate::ServiceRegistry::new();
        let _origin = crate::CallerOrigin::Client;
    }
}

