//! Integration tests for API layer hardening (SPEC-177):
//! CORS, request body limit, and error format.

use std::time::Duration;

use reqwest::header;
use topgun_server::network::NetworkConfig;
use topgun_server::network::NetworkModule;

/// Starts a minimal server on an OS-assigned port.
/// Returns `(port, shutdown_tx, serve_handle)`.
async fn start_server(config: NetworkConfig) -> (u16, tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let mut module = NetworkModule::new(config);
    let port = module.start().await.expect("start should succeed");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let serve_handle = tokio::spawn(async move {
        module
            .serve(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve should not fail");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    (port, shutdown_tx, serve_handle)
}

fn config_with_origin(origin: &str) -> NetworkConfig {
    NetworkConfig {
        cors_origins: vec![origin.to_string()],
        ..NetworkConfig::default()
    }
}

// ---------------------------------------------------------------------------
// CORS: preflight includes PUT and DELETE in allowed methods
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cors_preflight_allows_put_and_delete() {
    let (port, shutdown_tx, _) = start_server(config_with_origin("http://localhost:3000")).await;

    let client = reqwest::Client::new();
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("http://127.0.0.1:{port}/api/admin/settings"))
        .header("Origin", "http://localhost:3000")
        .header("Access-Control-Request-Method", "PUT")
        .send()
        .await
        .expect("preflight request should succeed");

    let allow_methods = resp
        .headers()
        .get("access-control-allow-methods")
        .expect("access-control-allow-methods header present")
        .to_str()
        .unwrap();

    assert!(allow_methods.contains("PUT"), "allowed methods should include PUT, got: {allow_methods}");
    assert!(allow_methods.contains("DELETE"), "allowed methods should include DELETE, got: {allow_methods}");
    assert!(allow_methods.contains("PATCH"), "allowed methods should include PATCH, got: {allow_methods}");

    drop(shutdown_tx);
}

// ---------------------------------------------------------------------------
// CORS: preflight includes Access-Control-Max-Age: 86400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cors_preflight_includes_max_age() {
    let (port, shutdown_tx, _) = start_server(config_with_origin("http://localhost:3000")).await;

    let client = reqwest::Client::new();
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("http://127.0.0.1:{port}/api/admin/settings"))
        .header("Origin", "http://localhost:3000")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("preflight request should succeed");

    let max_age = resp
        .headers()
        .get("access-control-max-age")
        .expect("access-control-max-age header present")
        .to_str()
        .unwrap();

    assert_eq!(max_age, "86400", "max-age should be 86400, got: {max_age}");

    drop(shutdown_tx);
}

// ---------------------------------------------------------------------------
// CORS: credentials header present with explicit origin
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cors_preflight_includes_credentials_with_explicit_origin() {
    let (port, shutdown_tx, _) = start_server(config_with_origin("http://localhost:3000")).await;

    let client = reqwest::Client::new();
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("http://127.0.0.1:{port}/api/admin/settings"))
        .header("Origin", "http://localhost:3000")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("preflight request should succeed");

    let credentials = resp
        .headers()
        .get("access-control-allow-credentials")
        .expect("access-control-allow-credentials header present")
        .to_str()
        .unwrap();

    assert_eq!(credentials, "true");

    drop(shutdown_tx);
}

// ---------------------------------------------------------------------------
// CORS: credentials NOT set with wildcard origin
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cors_no_credentials_with_wildcard_origin() {
    let (port, shutdown_tx, _) = start_server(config_with_origin("*")).await;

    let client = reqwest::Client::new();
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("http://127.0.0.1:{port}/api/status"))
        .header("Origin", "http://evil.com")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("preflight request should succeed");

    assert!(
        resp.headers().get("access-control-allow-credentials").is_none(),
        "credentials header must NOT be set with wildcard origin"
    );

    drop(shutdown_tx);
}

// ---------------------------------------------------------------------------
// Body limit: 3 MB POST to /sync returns 413
// ---------------------------------------------------------------------------

#[tokio::test]
async fn body_limit_rejects_oversized_sync_request() {
    let (port, shutdown_tx, _) = start_server(NetworkConfig::default()).await;

    let oversized_body = vec![0u8; 3 * 1024 * 1024]; // 3 MB

    let client = reqwest::Client::new();
    let result = client
        .post(format!("http://127.0.0.1:{port}/sync"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(oversized_body)
        .send()
        .await;

    // The server may either respond with 413 or reset the connection before
    // the full body is transmitted. Both indicate the limit is enforced.
    match result {
        Ok(resp) => assert_eq!(
            resp.status().as_u16(),
            413,
            "3 MB body should be rejected with 413, got: {}",
            resp.status()
        ),
        Err(e) => assert!(
            e.is_request(),
            "expected connection reset from body limit, got unexpected error: {e}"
        ),
    }

    drop(shutdown_tx);
}

// ---------------------------------------------------------------------------
// Body limit: 3 MB POST to admin endpoint returns 413
// ---------------------------------------------------------------------------

#[tokio::test]
async fn body_limit_rejects_oversized_admin_request() {
    let (port, shutdown_tx, _) = start_server(NetworkConfig::default()).await;

    let oversized_body = vec![0u8; 3 * 1024 * 1024]; // 3 MB

    let client = reqwest::Client::new();
    let result = client
        .post(format!("http://127.0.0.1:{port}/api/admin/policies"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(oversized_body)
        .send()
        .await;

    match result {
        Ok(resp) => assert_eq!(
            resp.status().as_u16(),
            413,
            "3 MB body should be rejected with 413, got: {}",
            resp.status()
        ),
        Err(e) => assert!(
            e.is_request(),
            "expected connection reset from body limit, got unexpected error: {e}"
        ),
    }

    drop(shutdown_tx);
}

// ---------------------------------------------------------------------------
// Body limit: request within limit is accepted (not rejected)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn body_limit_accepts_request_within_limit() {
    let (port, shutdown_tx, _) = start_server(NetworkConfig::default()).await;

    let small_body = vec![0u8; 1024]; // 1 KB

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/sync"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(small_body)
        .send()
        .await
        .expect("request should complete");

    // Should NOT be 413 — may be 400/422 due to invalid payload, but not 413.
    assert_ne!(
        resp.status().as_u16(),
        413,
        "1 KB body should not be rejected by body limit"
    );

    drop(shutdown_tx);
}
