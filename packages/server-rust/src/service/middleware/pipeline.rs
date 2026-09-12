//! Pipeline composition: combines all middleware layers into a single service stack.

use std::sync::Arc;

use tower::ServiceBuilder;

use super::authorization::AuthorizationLayer;
use super::load_shed::LoadShedLayer;
use super::metrics::MetricsLayer;
use super::timeout::TimeoutLayer;
use crate::service::config::ServerConfig;
use crate::service::operation::OperationPipeline;
// Imported for use by the test module (via `super::*`).
#[cfg(test)]
use crate::service::operation::{Operation, OperationError, OperationResponse};
use crate::service::policy::PolicyEvaluator;
use crate::service::router::OperationRouter;

/// Build the operation pipeline by wrapping the `OperationRouter` with middleware layers.
///
/// Layer order (outermost to innermost):
/// 1. `LoadShedLayer` -- reject when overloaded (fail fast before doing any work)
/// 2. `TimeoutLayer` -- enforce per-operation timeouts
/// 3. `MetricsLayer` -- record timing and outcome (before auth so denied requests are tracked)
/// 4. `AuthorizationLayer` (optional) -- RBAC policy enforcement; omitted when `None`
/// 5. `OperationRouter` -- domain service dispatch
///
/// When `policy_evaluator` is `None` (RBAC not configured), the authorization layer
/// is omitted entirely so there is zero overhead for non-RBAC deployments.
///
/// Returns a `BoxService` to erase the unnameable composed type, making the
/// pipeline storable in `AppState` via `Arc<Mutex<OperationPipeline>>`.
#[must_use]
pub fn build_operation_pipeline(
    router: OperationRouter,
    config: &ServerConfig,
    policy_evaluator: Option<Arc<PolicyEvaluator>>,
) -> OperationPipeline {
    if let Some(evaluator) = policy_evaluator {
        let svc = ServiceBuilder::new()
            .layer(LoadShedLayer::new(config.max_concurrent_operations))
            .layer(TimeoutLayer)
            .layer(MetricsLayer)
            .layer(AuthorizationLayer::new(evaluator))
            .service(router);
        OperationPipeline::new(svc)
    } else {
        let svc = ServiceBuilder::new()
            .layer(LoadShedLayer::new(config.max_concurrent_operations))
            .layer(TimeoutLayer)
            .layer(MetricsLayer)
            .service(router);
        OperationPipeline::new(svc)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use metrics_exporter_prometheus::PrometheusBuilder;
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

    /// Service that sleeps past any plausible budget before answering, so the
    /// timeout layer always wins the race in the timeout test.
    struct SleepingStub;

    impl Service<Operation> for SleepingStub {
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
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(OperationResponse::NotImplemented {
                    service_name: name,
                    call_id,
                })
            })
        }
    }

    fn make_op() -> Operation {
        make_op_with_timeout(5000)
    }

    fn make_op_with_timeout(call_timeout_ms: u64) -> Operation {
        let ctx = OperationContext::new(
            42,
            service_names::CRDT,
            Timestamp {
                millis: 0,
                counter: 0,
                node_id: "test".to_string(),
            },
            call_timeout_ms,
        );
        Operation::GarbageCollect { ctx }
    }

    /// Read one counter out of a Prometheus render by name plus a label subset.
    ///
    /// Matching on the parsed label set rather than on a raw substring keeps the
    /// assertion independent of the order the exporter happens to emit labels in,
    /// and returning `None` for an absent series is what lets a test distinguish
    /// "counted zero" from "never registered".
    fn rendered_counter(render: &str, name: &str, labels: &[(&str, &str)]) -> Option<u64> {
        'lines: for line in render.lines() {
            if line.starts_with('#') {
                continue;
            }
            let Some((head, value)) = line.rsplit_once(' ') else {
                continue;
            };
            let (line_name, label_blob) = match head.split_once('{') {
                Some((n, rest)) => (n, rest.trim_end_matches('}')),
                None => (head, ""),
            };
            if line_name != name {
                continue;
            }
            for (key, want) in labels {
                let needle = format!("{key}=\"{want}\"");
                if !label_blob.split(',').any(|pair| pair == needle) {
                    continue 'lines;
                }
            }
            let raw = value.trim();
            return raw.strip_suffix(".0").unwrap_or(raw).parse::<u64>().ok();
        }
        None
    }

    #[tokio::test]
    async fn pipeline_routes_through_all_layers() {
        let mut router = OperationRouter::new();
        router.register(service_names::CRDT, StubService);

        let config = ServerConfig {
            max_concurrent_operations: 100,
            ..ServerConfig::default()
        };

        let svc = build_operation_pipeline(router, &config, None);
        let resp = svc.oneshot(make_op()).await.unwrap();
        assert!(matches!(
            resp,
            OperationResponse::NotImplemented {
                service_name: "crdt",
                call_id: 42,
            }
        ));
    }

    /// An operation that exhausts its budget must be COUNTED, not merely refused.
    ///
    /// The recorder is a thread-local, so the runtime is built and driven inside
    /// the binding and everything is polled on that one current-thread runtime.
    /// The clock is paused: the runtime then auto-advances to the earliest
    /// deadline, which is the 50 ms budget rather than the stub's 200 ms sleep,
    /// so the outcome cannot depend on wall-clock scheduling.
    #[test]
    fn timed_out_operation_increments_the_timeout_error_counter() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        let rendered = metrics::with_local_recorder(&recorder, || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .start_paused(true)
                .build()
                .expect("current-thread runtime");
            rt.block_on(async {
                let mut router = OperationRouter::new();
                router.register(service_names::CRDT, SleepingStub);

                let config = ServerConfig {
                    max_concurrent_operations: 100,
                    ..ServerConfig::default()
                };

                let svc = build_operation_pipeline(router, &config, None);
                let err = svc.oneshot(make_op_with_timeout(50)).await.unwrap_err();
                assert!(
                    matches!(err, OperationError::Timeout { timeout_ms: 50 }),
                    "the budget must be what fails the call, got {err:?}"
                );
            });
            handle.render()
        });

        assert_eq!(
            rendered_counter(
                &rendered,
                "topgun_operation_errors_total",
                &[("service", "crdt"), ("error", "timeout")],
            ),
            Some(1),
            "a real timeout must reach the error counter; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_counter(
                &rendered,
                "topgun_operations_total",
                &[("service", "crdt"), ("outcome", "error")],
            ),
            Some(1),
            "a timed-out operation must count as an error outcome; render was:\n{rendered}"
        );
    }

    /// A shed operation must be COUNTED, not silently dropped.
    ///
    /// The permit is taken synchronously inside `call`, so holding the first
    /// future WITHOUT polling it is enough to occupy the only permit — no sleep
    /// and no spawn, and therefore no timing assumption at all.
    #[test]
    fn shed_operation_increments_the_overloaded_error_counter() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();

        let rendered = metrics::with_local_recorder(&recorder, || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime");
            rt.block_on(async {
                let mut router = OperationRouter::new();
                router.register(service_names::CRDT, StubService);

                let config = ServerConfig {
                    max_concurrent_operations: 1,
                    ..ServerConfig::default()
                };

                let mut svc = build_operation_pipeline(router, &config, None);

                // Created, deliberately not polled: it already owns the permit.
                let in_flight = svc.ready().await.unwrap().call(make_op());

                let shed = svc.ready().await.unwrap().call(make_op()).await;
                assert!(
                    matches!(shed, Err(OperationError::Overloaded)),
                    "the second operation must be shed, got {shed:?}"
                );

                let held = in_flight.await.unwrap();
                assert!(
                    matches!(
                        held,
                        OperationResponse::NotImplemented {
                            service_name: "crdt",
                            call_id: 42,
                        }
                    ),
                    "the permit holder must still complete normally"
                );
            });
            handle.render()
        });

        assert_eq!(
            rendered_counter(
                &rendered,
                "topgun_operation_errors_total",
                &[("service", "crdt"), ("error", "overloaded")],
            ),
            Some(1),
            "a real shed must reach the error counter; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_counter(
                &rendered,
                "topgun_operations_total",
                &[("service", "crdt"), ("outcome", "error")],
            ),
            Some(1),
            "a shed operation must count as an error outcome; render was:\n{rendered}"
        );
        assert_eq!(
            rendered_counter(
                &rendered,
                "topgun_operations_total",
                &[("service", "crdt"), ("outcome", "ok")],
            ),
            Some(1),
            "the permit holder must count as an ok outcome; render was:\n{rendered}"
        );
    }
}
