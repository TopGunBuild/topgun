//! HTTP middleware stack for the `TopGun` server.
//!
//! Builds the Tower middleware pipeline applied to all HTTP requests.
//! Middleware ordering follows the outer-to-inner convention: the first
//! layer listed is the outermost (processes the request first on the way
//! in, and the response last on the way out).

use axum::http::header::HeaderName;
use axum::http::{Method, StatusCode};
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use super::config::NetworkConfig;

/// The composed Tower layer type produced by [`build_http_layers`].
///
/// This type alias keeps the function signature readable. Each layer
/// wraps the next in a `Stack`, from outermost (first applied) to
/// innermost (last applied).
type HttpLayers = tower::layer::util::Stack<
    PropagateRequestIdLayer,
    tower::layer::util::Stack<
        TimeoutLayer,
        tower::layer::util::Stack<
            CorsLayer,
            tower::layer::util::Stack<
                CompressionLayer,
                tower::layer::util::Stack<
                    TraceLayer<
                        tower_http::classify::SharedClassifier<
                            tower_http::classify::ServerErrorsAsFailures,
                        >,
                    >,
                    tower::layer::util::Stack<
                        SetRequestIdLayer<MakeRequestUuid>,
                        tower::layer::util::Identity,
                    >,
                >,
            >,
        >,
    >,
>;

/// Builds the HTTP-level Tower middleware stack from the network configuration.
///
/// **Middleware ordering (outermost to innermost):**
/// 1. `SetRequestId` -- assigns a UUID v4 `X-Request-Id` to every incoming request
/// 2. `Tracing` -- logs request/response with structured trace spans
/// 3. `Compression` -- gzip response compression for bandwidth savings
/// 4. `CORS` -- Cross-Origin Resource Sharing based on configured origins
/// 5. `Timeout` -- enforces a maximum request processing duration
/// 6. `PropagateRequestId` -- copies `X-Request-Id` from the request to the response
///
/// This is transport-level middleware only. Operation-level middleware (metrics,
/// load shedding, auth, partition routing) belongs to a future service layer.
#[must_use]
pub fn build_http_layers(config: &NetworkConfig) -> HttpLayers {
    let x_request_id = HeaderName::from_static("x-request-id");

    let cors = build_cors_layer(&config.cors_origins);

    ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(PropagateRequestIdLayer::new(x_request_id))
        .into_inner()
}

/// Builds the CORS layer from the configured list of allowed origins.
///
/// A wildcard `"*"` in the origins list allows any origin. Otherwise,
/// each origin string is parsed and added to an explicit allowlist.
fn build_cors_layer(origins: &[String]) -> CorsLayer {
    let allow_origin = if origins.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        let parsed: Vec<_> = origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        AllowOrigin::list(parsed)
    };

    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn build_http_layers_does_not_panic_with_defaults() {
        let config = NetworkConfig::default();
        let _layers = build_http_layers(&config);
    }

    #[test]
    fn build_cors_layer_wildcard() {
        let origins = vec!["*".to_string()];
        let _cors = build_cors_layer(&origins);
    }

    #[test]
    fn build_cors_layer_specific_origins() {
        let origins = vec![
            "http://localhost:3000".to_string(),
            "https://example.com".to_string(),
        ];
        let _cors = build_cors_layer(&origins);
    }

    #[test]
    fn build_http_layers_with_custom_timeout() {
        let config = NetworkConfig {
            request_timeout: Duration::from_secs(5),
            ..NetworkConfig::default()
        };
        let _layers = build_http_layers(&config);
    }
}
