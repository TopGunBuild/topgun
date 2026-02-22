//! Timeout middleware for operations.
//!
//! Rejects operations that exceed their `call_timeout_ms` with `OperationError::Timeout`.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tower::{Layer, Service};

use crate::service::operation::{Operation, OperationError, OperationResponse};

// ---------------------------------------------------------------------------
// TimeoutLayer
// ---------------------------------------------------------------------------

/// Tower layer that wraps services with per-operation timeout enforcement.
///
/// The timeout is read from each operation's `ctx.call_timeout_ms` field,
/// allowing different operations to have different timeouts.
#[derive(Debug, Clone)]
pub struct TimeoutLayer;

impl<S> Layer<S> for TimeoutLayer {
    type Service = TimeoutService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TimeoutService { inner }
    }
}

// ---------------------------------------------------------------------------
// TimeoutService
// ---------------------------------------------------------------------------

/// Service wrapper that enforces per-operation timeouts.
#[derive(Debug, Clone)]
pub struct TimeoutService<S> {
    inner: S,
}

impl<S> Service<Operation> for TimeoutService<S>
where
    S: Service<Operation, Response = OperationResponse, Error = OperationError> + Send,
    S::Future: Send + 'static,
{
    type Response = OperationResponse;
    type Error = OperationError;
    type Future = Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let timeout_ms = op.ctx().call_timeout_ms;
        let fut = self.inner.call(op);
        Box::pin(async move {
            let duration = Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, fut).await {
                Ok(result) => result,
                Err(_elapsed) => Err(OperationError::Timeout { timeout_ms }),
            }
        })
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
    use crate::service::operation::OperationContext;

    /// Service that takes a configurable delay before responding.
    struct SlowService {
        delay_ms: u64,
    }

    impl Service<Operation> for SlowService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future =
            Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _op: Operation) -> Self::Future {
            let delay = self.delay_ms;
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                Ok(OperationResponse::Empty)
            })
        }
    }

    fn make_op(timeout_ms: u64) -> Operation {
        let ctx = OperationContext {
            call_id: 1,
            partition_id: None,
            service_name: "test",
            caller_origin: crate::service::CallerOrigin::Client,
            client_id: None,
            caller_node_id: None,
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            call_timeout_ms: timeout_ms,
        };
        Operation::GarbageCollect { ctx }
    }

    #[tokio::test]
    async fn completes_within_timeout() {
        let layer = TimeoutLayer;
        let svc = layer.layer(SlowService { delay_ms: 10 });
        let op = make_op(1000);
        let resp = svc.oneshot(op).await.unwrap();
        assert!(matches!(resp, OperationResponse::Empty));
    }

    #[tokio::test]
    async fn exceeds_timeout_returns_error() {
        let layer = TimeoutLayer;
        let svc = layer.layer(SlowService { delay_ms: 200 });
        let op = make_op(50);
        let err = svc.oneshot(op).await.unwrap_err();
        assert!(matches!(err, OperationError::Timeout { timeout_ms: 50 }));
    }
}
