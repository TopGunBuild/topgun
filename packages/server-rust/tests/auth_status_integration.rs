//! Behavioral tests for `GET /api/auth/status`.
//!
//! Verifies that the endpoint:
//! - returns HTTP 200 with `{ "authRequired": false }` when no JWT secret is set
//! - returns HTTP 200 with `{ "authRequired": true }` when a JWT secret is set
//! - is reachable without an Authorization header in both states
//! - never returns 500

use std::time::Duration;

use serde::Deserialize;
use serial_test::serial;
use topgun_server::network::NetworkConfig;
use topgun_server::network::NetworkModule;

#[derive(Debug, Deserialize)]
struct AuthStatusBody {
    #[serde(rename = "authRequired")]
    auth_required: bool,
}

async fn start_test_server(
    config: NetworkConfig,
) -> (
    u16,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let mut module = NetworkModule::new(config);
    let port = module.start().await.expect("server start should succeed");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        module
            .serve(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve should not fail");
    });

    // Give the server a moment to accept connections.
    tokio::time::sleep(Duration::from_millis(50)).await;

    (port, shutdown_tx, handle)
}

/// When no `JWT_SECRET` is set the server runs in no-auth mode.
/// The status endpoint must return 200 with authRequired=false without a token.
#[tokio::test]
#[serial]
async fn auth_status_returns_false_when_no_jwt_secret() {
    // Ensure JWT_SECRET is absent for this test.
    std::env::remove_var("JWT_SECRET");

    let (port, shutdown_tx, _handle) = start_test_server(NetworkConfig::default()).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/api/auth/status"))
        // No Authorization header — the endpoint must be public
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200, "status code must be 200");

    let body: AuthStatusBody = resp.json().await.expect("body must be valid JSON");
    assert!(
        !body.auth_required,
        "authRequired must be false when no JWT_SECRET is configured"
    );

    let _ = shutdown_tx.send(());
}

/// When `JWT_SECRET` is set the server requires authentication.
/// The status endpoint must return 200 with authRequired=true without a token.
#[tokio::test]
#[serial]
async fn auth_status_returns_true_when_jwt_secret_is_set() {
    std::env::set_var("JWT_SECRET", "test-secret-for-auth-status-test");

    let (port, shutdown_tx, _handle) = start_test_server(NetworkConfig::default()).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/api/auth/status"))
        // No Authorization header — must still return 200, not 401 or 500
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 200, "status code must be 200");

    let body: AuthStatusBody = resp.json().await.expect("body must be valid JSON");
    assert!(
        body.auth_required,
        "authRequired must be true when JWT_SECRET is configured"
    );

    let _ = shutdown_tx.send(());

    // Clean up the env var so it does not leak into subsequent tests.
    std::env::remove_var("JWT_SECRET");
}
