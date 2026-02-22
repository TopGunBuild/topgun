//! Domain service stubs.
//!
//! Each service implements both `ManagedService` (lifecycle) and `tower::Service<Operation>`
//! (request handling). All stubs return `OperationResponse::NotImplemented` -- actual
//! business logic will be implemented in per-domain specs.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tower::Service;

use crate::service::operation::{
    service_names, Operation, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};

// ---------------------------------------------------------------------------
// Macro for repetitive stub implementations
// ---------------------------------------------------------------------------

/// Generate a domain service stub with the given name and service constant.
macro_rules! domain_stub {
    (
        $(#[$meta:meta])*
        $name:ident, $svc_name:expr
    ) => {
        $(#[$meta])*
        pub struct $name;

        #[async_trait]
        impl ManagedService for $name {
            fn name(&self) -> &'static str {
                $svc_name
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

        impl Service<Operation> for Arc<$name> {
            type Response = OperationResponse;
            type Error = OperationError;
            type Future =
                Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }

            fn call(&mut self, op: Operation) -> Self::Future {
                let call_id = op.ctx().call_id;
                let name = $svc_name;
                Box::pin(async move {
                    Ok(OperationResponse::NotImplemented {
                        service_name: name,
                        call_id,
                    })
                })
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Domain service stubs
// ---------------------------------------------------------------------------

domain_stub!(
    /// CRDT domain service (LWW-Map and OR-Map operations).
    CrdtService, service_names::CRDT
);

domain_stub!(
    /// Sync domain service (Merkle tree synchronization).
    SyncService, service_names::SYNC
);

domain_stub!(
    /// Query domain service (live query subscriptions).
    QueryService, service_names::QUERY
);

domain_stub!(
    /// Messaging domain service (topic pub/sub).
    MessagingService, service_names::MESSAGING
);

domain_stub!(
    /// Coordination domain service (locks, partition map, heartbeat).
    CoordinationService, service_names::COORDINATION
);

domain_stub!(
    /// Search domain service (full-text search).
    SearchService, service_names::SEARCH
);

domain_stub!(
    /// Persistence domain service (counters, entry processing, journal, resolvers).
    PersistenceService, service_names::PERSISTENCE
);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::service::config::ServerConfig;
    use crate::service::operation::OperationContext;
    use crate::service::registry::ServiceRegistry;

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
    async fn crdt_service_returns_not_implemented() {
        let svc = Arc::new(CrdtService);
        let resp = svc.oneshot(make_op(service_names::CRDT)).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "crdt", .. }
        ));
    }

    #[tokio::test]
    async fn sync_service_returns_not_implemented() {
        let svc = Arc::new(SyncService);
        let resp = svc.oneshot(make_op(service_names::SYNC)).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "sync", .. }
        ));
    }

    #[tokio::test]
    async fn query_service_returns_not_implemented() {
        let svc = Arc::new(QueryService);
        let resp = svc.oneshot(make_op(service_names::QUERY)).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "query", .. }
        ));
    }

    #[tokio::test]
    async fn messaging_service_returns_not_implemented() {
        let svc = Arc::new(MessagingService);
        let resp = svc
            .oneshot(make_op(service_names::MESSAGING))
            .await
            .unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "messaging", .. }
        ));
    }

    #[tokio::test]
    async fn coordination_service_returns_not_implemented() {
        let svc = Arc::new(CoordinationService);
        let resp = svc
            .oneshot(make_op(service_names::COORDINATION))
            .await
            .unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "coordination", .. }
        ));
    }

    #[tokio::test]
    async fn search_service_returns_not_implemented() {
        let svc = Arc::new(SearchService);
        let resp = svc
            .oneshot(make_op(service_names::SEARCH))
            .await
            .unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "search", .. }
        ));
    }

    #[tokio::test]
    async fn persistence_service_returns_not_implemented() {
        let svc = Arc::new(PersistenceService);
        let resp = svc
            .oneshot(make_op(service_names::PERSISTENCE))
            .await
            .unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented { service_name: "persistence", .. }
        ));
    }

    #[tokio::test]
    async fn all_stubs_implement_managed_service() {
        let registry = ServiceRegistry::new();
        registry.register(CrdtService);
        registry.register(SyncService);
        registry.register(QueryService);
        registry.register(MessagingService);
        registry.register(CoordinationService);
        registry.register(SearchService);
        registry.register(PersistenceService);

        let ctx = ServiceContext {
            config: Arc::new(ServerConfig::default()),
        };
        registry.init_all(&ctx).await.unwrap();
        registry.shutdown_all(false).await.unwrap();

        // All services accessible by name.
        assert!(registry.get_by_name("crdt").is_some());
        assert!(registry.get_by_name("sync").is_some());
        assert!(registry.get_by_name("query").is_some());
        assert!(registry.get_by_name("messaging").is_some());
        assert!(registry.get_by_name("coordination").is_some());
        assert!(registry.get_by_name("search").is_some());
        assert!(registry.get_by_name("persistence").is_some());
    }
}
