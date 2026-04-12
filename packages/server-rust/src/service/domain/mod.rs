//! Domain service implementations.
//!
//! Each service implements both `ManagedService` (lifecycle) and `tower::Service<Operation>`
//! (request handling). All domain stubs have been replaced with real service implementations.

pub mod coordination;
pub use coordination::CoordinationService;

pub mod crdt;
pub use crdt::CrdtService;

pub mod sync;
pub use sync::SyncService;

pub mod messaging;
pub use messaging::MessagingService;

pub mod predicate;

pub mod query;

pub mod query_backend;
pub use query::QueryService;

pub mod counter;
pub mod journal;
pub mod persistence;
pub use persistence::PersistenceService;

pub mod schema;
pub use schema::SchemaService;

#[cfg(feature = "datafusion")]
pub mod arrow_cache;
#[cfg(feature = "datafusion")]
pub mod arrow_convert;
#[cfg(feature = "datafusion")]
pub mod datafusion_backend;
#[cfg(feature = "datafusion")]
pub mod table_provider;

pub mod search;
pub use search::SearchService;

pub mod index;

pub mod dag_dispatch;

pub mod embedding;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use parking_lot::RwLock;
    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::domain::search::SearchRegistry;
    use crate::service::operation::{service_names, Operation, OperationContext, OperationError};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::RecordStoreFactory;

    fn make_op(service_name: &'static str) -> Operation {
        let ctx = OperationContext::new(
            1,
            service_name,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        Operation::GarbageCollect { ctx }
    }

    #[tokio::test]
    async fn search_service_returns_wrong_service_for_non_search_ops() {
        let reg = Arc::new(SearchRegistry::new());
        let indexes = Arc::new(RwLock::new(HashMap::new()));
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let store_factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let needs_population = Arc::new(dashmap::DashMap::new());
        let svc = Arc::new(SearchService::new(
            reg,
            indexes,
            store_factory,
            conn_reg,
            needs_population,
        ));

        let err = svc
            .oneshot(make_op(service_names::SEARCH))
            .await
            .unwrap_err();
        assert!(matches!(err, OperationError::WrongService));
    }

    #[tokio::test]
    async fn persistence_service_returns_wrong_service_for_non_persistence_ops() {
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(PersistenceService::new(conn_reg, "test-node".to_string()));
        let err = svc
            .oneshot(make_op(service_names::PERSISTENCE))
            .await
            .unwrap_err();
        assert!(matches!(err, OperationError::WrongService));
    }
}
