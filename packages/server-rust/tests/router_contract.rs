//! Behavioral route-contract test for the `topgun-server` binary's HTTP router.
//!
//! Drives the real composed router (built via `admin_routes()` the same way the
//! binary does) through `tower::ServiceExt::oneshot` and asserts that every
//! route the admin SPA depends on returns a status from the allowed set
//! `{2xx, 401, 403, 405, 503}`. A 404 means the route is absent; a 500 means
//! a handler panicked or a required injection (e.g. `ConnectInfo`) is missing —
//! both are regressions that this test catches.

use std::net::SocketAddr;

use axum::extract::ConnectInfo;
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
    // /healthz is the readiness gate: 503 until WAL recovery completes, 200 after.
    // Registered once in admin_routes() so it surfaces on both the binary serve
    // path and NetworkModule; enumerated here so an admin_routes() refactor that
    // accidentally drops it fails red instead of silently regressing the gate.
    (Method::GET, "/healthz"),
    // /metrics is a live-verified binary 404 and a production-parity route;
    // included so binary and production router stay aligned even though the
    // admin SPA reaches metrics data via /api/status, not this endpoint directly.
    (Method::GET, "/metrics"),
];

/// Returns true when `status` is within the allowed response set for a known route.
///
/// Allowed: any 2xx, 401, 403, 405, 503.
/// Disallowed: 404 (route missing), 500 (handler panic / missing injection).
///
/// Note: `/api/auth/login` is checked with `is_allowed_status_login` instead because
/// that handler deliberately returns 500 when `TOPGUN_ADMIN_PASSWORD` is not set in
/// the test environment. That 500 is "not configured" behavior, not a regression.
fn is_allowed_status(status: StatusCode) -> bool {
    status.is_success()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status == StatusCode::METHOD_NOT_ALLOWED
        || status == StatusCode::SERVICE_UNAVAILABLE
}

/// Extended allowed set for the `/api/auth/login` route in the test fixture.
///
/// The login handler returns 500 when `TOPGUN_ADMIN_PASSWORD` is not set —
/// "not configured" behavior that predates this spec. All other statuses from
/// `is_allowed_status` remain valid; 500 is additionally permitted here.
fn is_allowed_status_login(status: StatusCode) -> bool {
    is_allowed_status(status) || status == StatusCode::INTERNAL_SERVER_ERROR
}

/// Build the binary's composed router the same way the binary does:
/// `admin_routes()` + the browser WS dual-mount on `/`.
/// Using the same construction path ensures the test exercises exactly the
/// router the binary serves, not a hypothetical variant.
fn build_binary_router(mount_admin: bool) -> axum::Router {
    // Use the default rate-limit values from NetworkConfig so the governor is
    // configured exactly as the binary would configure it.
    let default_config = topgun_server::network::config::NetworkConfig::default();
    admin_routes(
        default_config.rate_limit_per_ip,
        default_config.rate_limit_burst,
        mount_admin,
        default_config.trust_forwarded_for,
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

    let router = build_binary_router(true);

    // A real loopback address injected as ConnectInfo so the governor's
    // PeerIpKeyExtractor can identify the client IP without a real TCP connection.
    let peer_addr: SocketAddr = "127.0.0.1:12345".parse().expect("valid socket addr");

    for (method, path) in REQUIRED_ROUTES {
        // Routes that take a JSON body need content-type + a valid payload so
        // axum's Json extractor does not reject with 415/422 before reaching the handler.
        // login: both `username` and `password` are required non-Option fields.
        // update_settings: accepts any JSON object (empty `{}` is valid).
        let (body, content_type): (axum::body::Body, Option<&str>) =
            if *method == Method::POST && *path == "/api/auth/login" {
                (
                    axum::body::Body::from(r#"{"username":"admin","password":"test"}"#),
                    Some("application/json"),
                )
            } else if *method == Method::PUT && *path == "/api/admin/settings" {
                (axum::body::Body::from("{}"), Some("application/json"))
            } else {
                (axum::body::Body::empty(), None)
            };

        let mut builder = Request::builder().method(method.clone()).uri(*path);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        let mut req = builder.body(body).expect("request should build");
        req.extensions_mut().insert(ConnectInfo(peer_addr));

        let resp = router
            .clone()
            .oneshot(req)
            .await
            .expect("router should handle the request");

        let status = resp.status();
        // /api/auth/login returns 500 in the test fixture when TOPGUN_ADMIN_PASSWORD
        // is not set — that is intentional "not configured" behavior. All other
        // routes are held to the strict {2xx, 401, 403, 405, 503} allow-list.
        let allowed = if *path == "/api/auth/login" {
            is_allowed_status_login(status)
        } else {
            is_allowed_status(status)
        };
        assert!(
            allowed,
            "{method} {path} returned {status} — expected one of {{2xx, 401, 403, 405, 503}}. \
             A 404 means the route is missing from admin_routes() in network/module.rs; \
             a 500 means a handler error (e.g. missing ConnectInfo injection)."
        );
    }
}

#[tokio::test]
async fn admin_endpoints_bypass_auth_when_no_jwt_secret() {
    // AppState::for_test() has jwt_secret: None, which is the no-auth posture.
    // All /api/admin/* endpoints must return a non-401 status, proving that the
    // AdminClaims extractor synthesizes a local-admin identity instead of
    // rejecting the request when no JWT secret is configured.
    std::env::remove_var("JWT_SECRET");

    // mount_admin=true: this test's premise is that the admin plane IS mounted
    // under for_test() (no-auth) and the AdminClaims extractor synthesizes a
    // local-admin identity rather than 401ing. The gated-OFF posture is covered
    // separately by admin_plane_gated_off_returns_404.
    let router = build_binary_router(true);

    // GET endpoints: the store-less for_test() fixture returns 503 (not 401),
    // which proves the auth gate is bypassed.
    let no_auth_gets: &[(Method, &str)] = &[
        (Method::GET, "/api/admin/cluster/status"),
        (Method::GET, "/api/admin/settings"),
        (Method::GET, "/api/admin/maps"),
    ];

    // Inject ConnectInfo so the governor's PeerIpKeyExtractor doesn't 500.
    let peer_addr: SocketAddr = "127.0.0.1:12345".parse().expect("valid socket addr");

    for (method, path) in no_auth_gets {
        let mut req = Request::builder()
            .method(method.clone())
            .uri(*path)
            .body(axum::body::Body::empty())
            .expect("request should build");
        req.extensions_mut().insert(ConnectInfo(peer_addr));

        let resp = router
            .clone()
            .oneshot(req)
            .await
            .expect("router should handle the request");

        let status = resp.status();
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} returned 401 on no-auth server — \
             AdminClaims extractor should synthesize local-admin when jwt_secret is None"
        );
    }

    // PUT /api/admin/settings: mutating path must also bypass auth on no-auth server.
    // This proves Option 1 (full bypass at the extractor) works for write paths,
    // not just reads — the reason Option 1 was chosen over a read-only bypass.
    let mut req = Request::builder()
        .method(Method::PUT)
        .uri("/api/admin/settings")
        .header("content-type", "application/json")
        .body(axum::body::Body::from("{}"))
        .expect("request should build");
    req.extensions_mut().insert(ConnectInfo(peer_addr));

    let resp = router
        .oneshot(req)
        .await
        .expect("router should handle the request");

    let status = resp.status();
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "PUT /api/admin/settings returned 401 on no-auth server — \
         AdminClaims extractor should synthesize local-admin for mutating paths too"
    );
}

#[tokio::test]
async fn admin_plane_gated_off_returns_404() {
    // mount_admin=false is the no-auth public-bind posture: the admin control
    // plane must be ABSENT (404), not merely auth-guarded. A 404 proves the
    // synthesized superuser cannot be reached unauthenticated from the network;
    // a 401 would mean the route is still mounted (a regression). The data plane
    // (/health) must stay live (200) so the server is still useful.
    std::env::remove_var("JWT_SECRET");

    let router = build_binary_router(false);

    // ConnectInfo is injected for parity with the other tests even though the
    // governor layer is absent when the admin plane is gated off.
    let peer_addr: SocketAddr = "127.0.0.1:12345".parse().expect("valid socket addr");

    // Every gated-off path must return 404 (route absent), NOT 401 (mounted+guarded).
    let gated_off: &[(Method, &str, Option<&str>)] = &[
        (Method::GET, "/api/admin/cluster/status", None),
        (Method::GET, "/api/admin/maps", None),
        (Method::POST, "/api/auth/login", Some("application/json")),
    ];

    for (method, path, content_type) in gated_off {
        let body: axum::body::Body = if *method == Method::POST {
            axum::body::Body::from(r#"{"username":"admin","password":"test"}"#)
        } else {
            axum::body::Body::empty()
        };
        let mut builder = Request::builder().method(method.clone()).uri(*path);
        if let Some(ct) = content_type {
            builder = builder.header("content-type", *ct);
        }
        let mut req = builder.body(body).expect("request should build");
        req.extensions_mut().insert(ConnectInfo(peer_addr));

        let resp = router
            .clone()
            .oneshot(req)
            .await
            .expect("router should handle the request");

        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} {path} should be 404 when mount_admin=false (admin plane absent), \
             not {} — a non-404 means the route is still mounted and the gate failed.",
            resp.status()
        );
    }

    // The data plane stays live: /health must return 200 even with admin gated off.
    let mut req = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(axum::body::Body::empty())
        .expect("request should build");
    req.extensions_mut().insert(ConnectInfo(peer_addr));

    let resp = router
        .oneshot(req)
        .await
        .expect("router should handle the request");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/health should be 200 with admin gated off — the data plane must stay live"
    );
}
