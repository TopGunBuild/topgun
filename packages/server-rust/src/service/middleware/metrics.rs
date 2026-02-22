//! Metrics middleware for operations.
//!
//! Records operation duration and increments counters using `tracing` spans,
//! not a full metrics crate. Future enhancement: add prometheus/metrics crate.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use tower::{Layer, Service};
use tracing::{info_span, Instrument};

use crate::service::operation::{Operation, OperationError, OperationResponse};

// ---------------------------------------------------------------------------
// MetricsLayer
// ---------------------------------------------------------------------------

/// Tower layer that instruments operations with timing and counting via `tracing` spans.
#[derive(Debug, Clone)]
pub struct MetricsLayer;

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { inner }
    }
}

// ---------------------------------------------------------------------------
// MetricsService
// ---------------------------------------------------------------------------

/// Service wrapper that records operation duration and outcome in tracing spans.
#[derive(Debug, Clone)]
pub struct MetricsService<S> {
    inner: S,
}

impl<S> Service<Operation> for MetricsService<S>
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
        let service_name = op.ctx().service_name;
        let call_id = op.ctx().call_id;

        let span = info_span!(
            "operation",
            service = service_name,
            call_id = call_id,
            duration_ms = tracing::field::Empty,
            outcome = tracing::field::Empty,
        );

        let fut = self.inner.call(op);

        Box::pin(
            async move {
                let start = Instant::now();
                let result = fut.await;
                let duration_ms = start.elapsed().as_millis();

                let outcome = match &result {
                    Ok(_) => "ok",
                    Err(_) => "error",
                };

                #[allow(clippy::cast_possible_truncation)]
                let duration_u64 = duration_ms as u64;
                tracing::Span::current().record("duration_ms", duration_u64);
                tracing::Span::current().record("outcome", outcome);

                tracing::info!(
                    service = service_name,
                    call_id = call_id,
                    duration_ms = duration_u64,
                    outcome = outcome,
                    "operation complete"
                );

                result
            }
            .instrument(span),
        )
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

    /// Immediately-completing service for metrics testing.
    struct ImmediateService;

    impl Service<Operation> for ImmediateService {
        type Response = OperationResponse;
        type Error = OperationError;
        type Future =
            Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, op: Operation) -> Self::Future {
            let call_id = op.ctx().call_id;
            Box::pin(async move { Ok(OperationResponse::Ack { call_id }) })
        }
    }

    #[tokio::test]
    async fn metrics_layer_passes_through_response() {
        let layer = MetricsLayer;
        let svc = layer.layer(ImmediateService);

        let ctx = OperationContext::new(
            42,
            "crdt",
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            5000,
        );
        let op = Operation::GarbageCollect { ctx };

        let resp = svc.oneshot(op).await.unwrap();
        assert!(matches!(resp, OperationResponse::Ack { call_id: 42 }));
    }
}
