//! Operation routing: dispatches `Operation` to domain services by `service_name`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tower::Service;

use super::operation::{Operation, OperationError, OperationResponse};

// ---------------------------------------------------------------------------
// DomainHandler trait alias
// ---------------------------------------------------------------------------

/// A boxed Tower service that handles operations for a single domain.
type BoxedService =
    Box<dyn Service<Operation, Response = OperationResponse, Error = OperationError, Future = BoxedFuture> + Send>;

type BoxedFuture = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

// ---------------------------------------------------------------------------
// OperationRouter
// ---------------------------------------------------------------------------

/// Routes `Operation` values to the correct domain service by `service_name`.
///
/// Each registered domain service is a `tower::Service<Operation>` keyed by
/// its service name (e.g., `"crdt"`, `"sync"`). Operations with an unregistered
/// `service_name` return `OperationError::UnknownService`.
pub struct OperationRouter {
    services: HashMap<&'static str, BoxedService>,
}

impl OperationRouter {
    /// Create a new empty router.
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    /// Register a domain service for the given name.
    pub fn register<S>(&mut self, name: &'static str, service: S)
    where
        S: Service<Operation, Response = OperationResponse, Error = OperationError> + Send + 'static,
        S::Future: Send + 'static,
    {
        self.services.insert(name, Box::new(ServiceWrapper(service)));
    }
}

impl Default for OperationRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Service<Operation> for OperationRouter {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // All registered services must be ready. If any is not ready, return pending.
        for svc in self.services.values_mut() {
            match svc.poll_ready(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let service_name = op.ctx().service_name;
        match self.services.get_mut(service_name) {
            Some(svc) => svc.call(op),
            None => Box::pin(async move {
                Err(OperationError::UnknownService {
                    name: service_name.to_string(),
                })
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// ServiceWrapper (type-erased adapter)
// ---------------------------------------------------------------------------

/// Wrapper to type-erase a concrete `Service<Operation>` into a `BoxedService`.
struct ServiceWrapper<S>(S);

impl<S> Service<Operation> for ServiceWrapper<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError> + Send,
    S::Future: Send + 'static,
{
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = BoxedFuture;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        Box::pin(self.0.call(op))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::service::operation::{service_names, OperationContext};

    /// A simple stub service that returns `NotImplemented` for any operation.
    #[derive(Clone)]
    struct StubService {
        name: &'static str,
    }

    impl Service<Operation> for StubService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            let name = self.name;
            let call_id = op.ctx().call_id;
            Box::pin(async move {
                Ok(OperationResponse::NotImplemented {
                    service_name: name,
                    call_id,
                })
            })
        }
    }

    fn make_ctx(service_name: &'static str) -> OperationContext {
        OperationContext::new(
            1,
            service_name,
            Timestamp {
                millis: 1_700_000_000_000,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        )
    }

    #[tokio::test]
    async fn routes_to_registered_service() {
        let mut router = OperationRouter::new();
        router.register(service_names::COORDINATION, StubService { name: "coordination" });

        let ctx = make_ctx(service_names::COORDINATION);
        let op = Operation::GarbageCollect { ctx };

        let resp = router.oneshot(op).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "coordination",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn unknown_service_returns_error() {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, StubService { name: "crdt" });

        let ctx = make_ctx("nonexistent");
        let op = Operation::GarbageCollect { ctx };

        let err = router.oneshot(op).await.unwrap_err();
        assert!(matches!(
            err,
            OperationError::UnknownService { name } if name == "nonexistent"
        ));
    }

    #[tokio::test]
    async fn routes_to_correct_service_among_multiple() {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, StubService { name: "crdt" });
        router.register(service_names::SYNC, StubService { name: "sync" });
        router.register(service_names::QUERY, StubService { name: "query" });

        // Route to sync
        let ctx = make_ctx(service_names::SYNC);
        let op = Operation::GarbageCollect { ctx };
        let resp = ServiceExt::ready(&mut router).await.unwrap().call(op).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "sync",
                ..
            }
        ));

        // Route to query
        let ctx = make_ctx(service_names::QUERY);
        let op = Operation::GarbageCollect { ctx };
        let resp = ServiceExt::ready(&mut router).await.unwrap().call(op).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "query",
                ..
            }
        ));
    }
}
