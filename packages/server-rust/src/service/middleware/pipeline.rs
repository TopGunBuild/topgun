//! Pipeline composition: combines all middleware layers into a single service stack.

use tower::ServiceBuilder;

use super::load_shed::LoadShedLayer;
use super::metrics::MetricsLayer;
use super::timeout::TimeoutLayer;
use crate::service::config::ServerConfig;
use crate::service::operation::{Operation, OperationError, OperationResponse};
use crate::service::router::OperationRouter;

/// Build the operation pipeline by wrapping the `OperationRouter` with middleware layers.
///
/// Layer order (outermost to innermost):
/// 1. `LoadShedLayer` -- reject when overloaded (fail fast before doing any work)
/// 2. `TimeoutLayer` -- enforce per-operation timeouts
/// 3. `MetricsLayer` -- record timing and outcome (closest to the actual handler)
///
/// The returned service implements `tower::Service<Operation>`.
#[must_use]
pub fn build_operation_pipeline(
    router: OperationRouter,
    config: &ServerConfig,
) -> impl tower::Service<Operation, Response = OperationResponse, Error = OperationError> {
    ServiceBuilder::new()
        .layer(LoadShedLayer::new(config.max_concurrent_operations))
        .layer(TimeoutLayer)
        .layer(MetricsLayer)
        .service(router)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use topgun_core::Timestamp;
    use tower::{Service, ServiceExt};

    use super::*;
    use crate::service::operation::{service_names, OperationContext};

    /// Stub service for pipeline testing.
    struct StubService;

    impl Service<Operation> for StubService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future =
            Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            let call_id = op.ctx().call_id;
            let name = op.ctx().service_name;
            Box::pin(async move {
                Ok(OperationResponse::NotImplemented {
                    service_name: name,
                    call_id,
                })
            })
        }
    }

    fn make_op() -> Operation {
        let ctx = OperationContext::new(
            42,
            service_names::CRDT,
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
    async fn pipeline_routes_through_all_layers() {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, StubService);

        let config = ServerConfig {
            max_concurrent_operations: 100,
            ..ServerConfig::default()
        };

        let svc = build_operation_pipeline(router, &config);
        let resp = svc.oneshot(make_op()).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "crdt",
                call_id: 42,
            }
        ));
    }
}
