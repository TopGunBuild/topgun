//! Load-shedding middleware for operations.
//!
//! Rejects operations when the server is overloaded (concurrent count exceeds
//! `max_concurrent_operations`) with `OperationError::Overloaded`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::sync::Semaphore;
use tower::{Layer, Service};

use crate::service::operation::{Operation, OperationError, OperationResponse};

// ---------------------------------------------------------------------------
// LoadShedLayer
// ---------------------------------------------------------------------------

/// Tower layer that limits concurrent operations via a semaphore.
///
/// When all permits are taken, incoming operations are rejected immediately
/// with `OperationError::Overloaded` rather than queued.
#[derive(Debug, Clone)]
pub struct LoadShedLayer {
    semaphore: Arc<Semaphore>,
}

impl LoadShedLayer {
    /// Create a new `LoadShedLayer` with the given concurrency limit.
    #[must_use]
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent as usize)),
        }
    }
}

impl<S> Layer<S> for LoadShedLayer {
    type Service = LoadShedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LoadShedService {
            inner,
            semaphore: self.semaphore.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// LoadShedService
// ---------------------------------------------------------------------------

/// Service wrapper that enforces a concurrency limit via semaphore-based backpressure.
#[derive(Debug, Clone)]
pub struct LoadShedService<S> {
    inner: S,
    semaphore: Arc<Semaphore>,
}

impl<S> Service<Operation> for LoadShedService<S>
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
        // Try to acquire a permit without waiting. If none available, reject.
        let Ok(permit) = self.semaphore.clone().try_acquire_owned() else {
            return Box::pin(async { Err(OperationError::Overloaded) });
        };

        let fut = self.inner.call(op);
        Box::pin(async move {
            // Hold the permit for the duration of the operation.
            let result = fut.await;
            drop(permit);
            result
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::service::operation::OperationContext;

    /// Service that holds for a configurable duration.
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

    fn make_op() -> Operation {
        let ctx = OperationContext::new(
            1,
            "test",
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
    async fn allows_operations_under_limit() {
        let layer = LoadShedLayer::new(10);
        let svc = layer.layer(SlowService { delay_ms: 1 });
        let resp = svc.oneshot(make_op()).await.unwrap();
        assert!(matches!(resp, OperationResponse::Empty));
    }

    #[tokio::test]
    async fn rejects_when_overloaded() {
        let layer = LoadShedLayer::new(1);
        let mut svc = layer.layer(SlowService { delay_ms: 500 });

        // First operation acquires the single permit.
        let _ = ServiceExt::ready(&mut svc).await.unwrap();
        let _in_flight = tokio::spawn({
            let fut = svc.call(make_op());
            async move { fut.await }
        });

        // Give the spawned task time to acquire the permit.
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Second operation should be rejected immediately.
        let err = svc.call(make_op()).await.unwrap_err();
        assert!(matches!(err, OperationError::Overloaded));
    }
}
