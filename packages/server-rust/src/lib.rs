//! `TopGun` Server — `WebSocket` server with clustering, partitioning, and `PostgreSQL` storage.

pub mod cluster;
pub mod dag;
pub mod network;
pub mod service;
pub mod storage;
pub mod traits;

#[cfg(feature = "simulation")]
pub mod sim;

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

    use crate::cluster::state::ClusterState;
    use crate::cluster::types::ClusterConfig;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::config::ServerConfig;
    use crate::service::domain::{
        CoordinationService, CrdtService, MessagingService, PersistenceService, QueryService,
        SchemaService, SearchService, SyncService,
    };
    use crate::service::domain::search::SearchRegistry;
    use crate::service::domain::query::QueryRegistry;
    use crate::service::middleware::build_operation_pipeline;
    use crate::service::operation::{service_names, CallerOrigin, OperationResponse};
    use crate::service::registry::{ServiceContext, ServiceRegistry};
    use crate::service::router::OperationRouter;
    use crate::service::security::{SecurityConfig, WriteValidator};
    use crate::service::{ClassifyError, OperationService};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::{ObserverFactory, RecordStoreFactory};
    use crate::storage::impls::StorageConfig;
    use crate::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
    fn make_write_validator(node_id: &str) -> Arc<WriteValidator> {
        let hlc = Arc::new(parking_lot::Mutex::new(HLC::new(
            node_id.to_string(),
            Box::new(topgun_core::SystemClock),
        )));
        Arc::new(WriteValidator::new(Arc::new(SecurityConfig::default()), hlc))
    }

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

        // CoordinationService requires real dependencies.
        let cluster_config = Arc::new(ClusterConfig::default());
        let (cluster_state, _rx) =
            ClusterState::new(cluster_config, "integration-test-node".to_string());
        let cluster_state = Arc::new(cluster_state);
        let connection_registry = Arc::new(ConnectionRegistry::new());

        // MerkleSyncManager must be created before RecordStoreFactory so the
        // MerkleObserverFactory can be included in with_observer_factories().
        let merkle_manager = Arc::new(MerkleSyncManager::default());

        let merkle_observer_factory: Arc<dyn ObserverFactory> =
            Arc::new(MerkleObserverFactory::new(Arc::clone(&merkle_manager)));

        #[allow(unused_mut)]
        let mut observer_factories: Vec<Arc<dyn ObserverFactory>> =
            vec![merkle_observer_factory];

        // When datafusion is enabled, register ArrowCacheObserverFactory so that
        // record mutations invalidate the Arrow cache for SQL query freshness.
        #[cfg(feature = "datafusion")]
        let _arrow_cache_manager = {
            let mgr = Arc::new(
                crate::service::domain::arrow_cache::ArrowCacheManager::new(),
            );
            observer_factories.push(Arc::new(
                crate::service::domain::arrow_cache::ArrowCacheObserverFactory::new(
                    Arc::clone(&mgr),
                ),
            ));
            mgr
        };

        // Register IndexObserverFactory so that maps with secondary indexes
        // keep their indexes in sync with record mutations.
        let index_observer_factory = Arc::new(
            crate::service::domain::index::IndexObserverFactory::new(),
        );
        observer_factories.push(Arc::clone(&index_observer_factory) as Arc<dyn ObserverFactory>);

        let record_store_factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_observer_factories(observer_factories),
        );

        let query_registry = Arc::new(QueryRegistry::new());

        let mut router = OperationRouter::new();
        router.register(
            service_names::CRDT,
            Arc::new(CrdtService::new(
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
                make_write_validator(&config.node_id),
                Arc::clone(&query_registry),
                Arc::new(SchemaService::new()),
            )),
        );
        router.register(
            service_names::SYNC,
            Arc::new(SyncService::new(
                merkle_manager,
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
            )),
        );
        let query_merkle_manager = Arc::new(
            crate::storage::query_merkle::QueryMerkleSyncManager::new(),
        );
        router.register(
            service_names::QUERY,
            Arc::new(QueryService::new(
                Arc::clone(&query_registry),
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
                Arc::new(crate::service::domain::query_backend::PredicateBackend),
                Some(query_merkle_manager),
                config.max_query_records,
                Some(Arc::clone(&index_observer_factory)),
                #[cfg(feature = "datafusion")]
                None,
            )),
        );
        router.register(
            service_names::MESSAGING,
            Arc::new(MessagingService::new(Arc::clone(&connection_registry))),
        );
        router.register(
            service_names::COORDINATION,
            Arc::new(CoordinationService::new(
                cluster_state,
                Arc::clone(&connection_registry),
            )),
        );
        let search_needs_population = Arc::new(dashmap::DashMap::new());
        router.register(
            service_names::SEARCH,
            Arc::new(SearchService::new(
                Arc::new(SearchRegistry::new()),
                Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
                Arc::clone(&record_store_factory),
                Arc::clone(&connection_registry),
                search_needs_population,
            )),
        );
        router.register(
            service_names::PERSISTENCE,
            Arc::new(PersistenceService::new(
                connection_registry,
                "integration-test-node".to_string(),
            )),
        );

        (classify_svc, router, config)
    }

    #[tokio::test]
    async fn full_pipeline_ping_returns_pong() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config, None);

        // Classify a Ping message.
        let msg = Message::Ping(topgun_core::messages::PingData {
            timestamp: 1_700_000_000_000,
        });
        let op = classify_svc
            .classify(msg, Some("client-1".to_string()), CallerOrigin::Client)
            .unwrap();

        assert_eq!(op.ctx().service_name, service_names::COORDINATION);

        // Route through the full pipeline — real CoordinationService returns Pong.
        let resp = ServiceExt::ready(&mut pipeline)
            .await
            .unwrap()
            .call(op)
            .await
            .unwrap();

        match resp {
            OperationResponse::Message(msg) => {
                assert!(
                    matches!(*msg, Message::Pong(ref pong) if pong.timestamp == 1_700_000_000_000),
                    "expected Pong with echoed timestamp, got {msg:?}"
                );
            }
            other => panic!("expected Message(Pong), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn full_pipeline_client_op_to_op_ack() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config, None);

        let msg = Message::ClientOp(topgun_core::messages::sync::ClientOpMessage {
            payload: topgun_core::ClientOp {
                id: Some("op-1".to_string()),
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

        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::OpAck(_))),
            "expected OpAck, got {resp:?}"
        );
    }

    #[tokio::test]
    async fn full_pipeline_topic_subscribe_returns_empty() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config, None);

        let msg = Message::TopicSub {
            payload: topgun_core::messages::TopicSubPayload {
                topic: "chat.general".to_string(),
            },
        };
        let mut op = classify_svc
            .classify(msg, None, CallerOrigin::Client)
            .unwrap();

        assert_eq!(op.ctx().service_name, service_names::MESSAGING);

        // MessagingService requires a connection_id; set one that exists in the registry.
        // Since we don't have a real connection registered here, we expect an Internal error
        // when connection_id is None. Instead, just test that the routing works by providing
        // a connection_id -- the service will handle it gracefully even if the connection
        // doesn't exist in the registry.
        op = match op {
            crate::service::operation::Operation::TopicSubscribe { mut ctx, payload } => {
                ctx.connection_id = Some(crate::network::connection::ConnectionId(999));
                crate::service::operation::Operation::TopicSubscribe { ctx, payload }
            }
            other => other,
        };

        let resp = ServiceExt::ready(&mut pipeline)
            .await
            .unwrap()
            .call(op)
            .await
            .unwrap();

        assert!(
            matches!(resp, OperationResponse::Empty),
            "expected Empty from real MessagingService, got {resp:?}"
        );
    }

    /// AC13: SyncService replaces stub in full pipeline — SyncInit returns SyncRespRoot.
    #[tokio::test]
    async fn full_pipeline_sync_init_returns_sync_resp_root() {
        let (classify_svc, router, config) = setup();
        let mut pipeline = build_operation_pipeline(router, &config, None);

        let msg = Message::SyncInit(topgun_core::messages::SyncInitMessage {
            map_name: "users".to_string(),
            last_sync_timestamp: None,
        });
        let op = classify_svc
            .classify(msg, Some("client-sync".to_string()), CallerOrigin::Client)
            .unwrap();

        assert_eq!(op.ctx().service_name, service_names::SYNC);

        let resp = ServiceExt::ready(&mut pipeline)
            .await
            .unwrap()
            .call(op)
            .await
            .unwrap();

        assert!(
            matches!(resp, OperationResponse::Message(ref msg) if matches!(**msg, Message::SyncRespRoot(_))),
            "expected SyncRespRoot from full pipeline, got {resp:?}"
        );
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
        // CoordinationService requires constructor args, so it is registered
        // separately from the unit-struct stubs.
        let cluster_config = Arc::new(ClusterConfig::default());
        let (cluster_state, _rx) =
            ClusterState::new(cluster_config, "registry-test-node".to_string());
        let cluster_state = Arc::new(cluster_state);
        let connection_registry = Arc::new(ConnectionRegistry::new());

        let connection_registry_for_crdt = Arc::new(ConnectionRegistry::new());
        let record_store_factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let merkle_manager_for_sync = Arc::new(MerkleSyncManager::default());
        let connection_registry_for_sync = Arc::new(ConnectionRegistry::new());

        let query_registry = Arc::new(QueryRegistry::new());

        let registry = ServiceRegistry::new();
        registry.register(CrdtService::new(
            Arc::clone(&record_store_factory),
            connection_registry_for_crdt,
            make_write_validator("registry-test-node"),
            Arc::clone(&query_registry),
            Arc::new(SchemaService::new()),
        ));
        registry.register(SyncService::new(
            merkle_manager_for_sync,
            Arc::clone(&record_store_factory),
            connection_registry_for_sync,
        ));
        registry.register(QueryService::new(
            query_registry,
            Arc::clone(&record_store_factory),
            Arc::new(ConnectionRegistry::new()),
            Arc::new(crate::service::domain::query_backend::PredicateBackend),
            None,
            10_000,
            None,
            #[cfg(feature = "datafusion")]
            None,
        ));
        registry.register(MessagingService::new(Arc::clone(&connection_registry)));
        registry.register(CoordinationService::new(
            cluster_state,
            Arc::clone(&connection_registry),
        ));
        registry.register(SearchService::new(
            Arc::new(SearchRegistry::new()),
            Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
            Arc::clone(&record_store_factory),
            Arc::clone(&connection_registry),
            Arc::new(dashmap::DashMap::new()),
        ));
        registry.register(PersistenceService::new(
            connection_registry,
            "test-node".to_string(),
        ));

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

        // Also accessible by name.
        assert!(registry.get_by_name("coordination").is_some());

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

