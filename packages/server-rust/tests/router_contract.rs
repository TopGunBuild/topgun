//! Behavioral route-contract test for the `topgun-server` binary's HTTP router.
//!
//! Drives the real composed router (built via `admin_routes()` the same way the
//! binary does) through `tower::ServiceExt::oneshot` and asserts that every
//! route the admin SPA depends on returns non-404.
//!
//! The assertion is "route exists" (any non-404 status is a pass — 200, 401,
//! 403, 405, or 503 are all acceptable). A 404 means the route is absent from
//! the binary's router and fails the test.

use http::{Method, Request, StatusCode};
use topgun_server::network::handlers::AppState;
use topgun_server::network::module::admin_routes;
use tower::ServiceExt;

/// Single edit-point for the binary↔SPA route contract.
///
/// Each entry is `(HTTP method, path)`. Adding a new admin SPA fetch target
/// is a one-line change here; the test will fail red until the binary serves it.
/// Deleting any entry from `admin_routes()` will cause the corresponding test
/// case to return 404 and fail — proving the anti-regression guarantee.
const REQUIRED_ROUTES: &[(Method, &str)] = &[
    (Method::POST, "/api/auth/login"),
    (Method::GET, "/api/auth/status"),
    (Method::GET, "/api/status"),
    (Method::GET, "/api/admin/cluster/status"),
    (Method::GET, "/api/admin/settings"),
    (Method::PUT, "/api/admin/settings"),
    (Method::GET, "/api/admin/maps"),
    // /metrics is a live-verified binary 404 and a production-parity route;
    // included so binary and production router stay aligned even though the
    // admin SPA reaches metrics data via /api/status, not this endpoint directly.
    (Method::GET, "/metrics"),
];

/// Build the binary's composed router the same way the binary does:
/// `admin_routes()` + the browser WS dual-mount on `/`.
/// Using the same construction path ensures the test exercises exactly the
/// router the binary serves, not a hypothetical variant.
fn build_binary_router() -> axum::Router {
    // Use the default rate-limit values from NetworkConfig so the governor is
    // configured exactly as the binary would configure it.
    let default_config = topgun_server::network::config::NetworkConfig::default();
    admin_routes(
        default_config.rate_limit_per_ip,
        default_config.rate_limit_burst,
    )
    // Browser WS dual-mount: the binary's only extra route beyond admin_routes().
    .route(
        "/",
        axum::routing::get(topgun_server::network::handlers::ws_upgrade_handler),
    )
    .with_state(AppState::for_test())
}

#[tokio::test]
async fn binary_router_serves_all_required_routes() {
    // Ensure no JWT_SECRET is set so no-auth endpoints respond normally.
    std::env::remove_var("JWT_SECRET");

    let router = build_binary_router();

    for (method, path) in REQUIRED_ROUTES {
        let req = Request::builder()
            .method(method.clone())
            .uri(*path)
            .body(axum::body::Body::empty())
            .expect("request should build");

        let resp = router
            .clone()
            .oneshot(req)
            .await
            .expect("router should handle the request");

        let status = resp.status();
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} returned 404 — route is missing from the binary's router. \
             Add it to admin_routes() in packages/server-rust/src/network/module.rs"
        );
    }
}
