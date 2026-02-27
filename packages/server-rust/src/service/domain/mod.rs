//! Domain service implementations and stubs.
//!
//! Each service implements both `ManagedService` (lifecycle) and `tower::Service<Operation>`
//! (request handling). Stubs return `OperationResponse::NotImplemented` -- actual
//! business logic is implemented in per-domain modules as they are developed.

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
pub use query::QueryService;

pub mod counter;
pub mod journal;
pub mod persistence;
pub use persistence::PersistenceService;

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
    /// Search domain service (full-text search).
    SearchService, service_names::SEARCH
);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::network::connection::ConnectionRegistry;
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
    async fn persistence_service_returns_wrong_service_for_non_persistence_ops() {
        let conn_reg = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(PersistenceService::new(conn_reg, "test-node".to_string()));
        let err = svc
            .oneshot(make_op(service_names::PERSISTENCE))
            .await
            .unwrap_err();
        assert!(matches!(err, OperationError::WrongService));
    }

    #[tokio::test]
    async fn all_stubs_implement_managed_service() {
        // CoordinationService, CrdtService, SyncService, MessagingService,
        // QueryService, and PersistenceService are excluded: they require
        // constructor args and have dedicated tests in their own modules.
        let registry = ServiceRegistry::new();
        registry.register(SearchService);

        let ctx = ServiceContext {
            config: Arc::new(ServerConfig::default()),
        };
        registry.init_all(&ctx).await.unwrap();
        registry.shutdown_all(false).await.unwrap();

        // Stub services accessible by name.
        assert!(registry.get_by_name("search").is_some());
    }
}
