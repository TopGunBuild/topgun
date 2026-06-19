//! Behavioral integration tests for the pre-auth inbound-frame `DoS` hardening
//! (network audit F1 + F3).
//!
//! F1: the WS reader decoded every inbound frame with `rmp_serde` BEFORE auth.
//! `rmp_serde` has no recursion-depth limit and the message types are recursive,
//! so a deeply-nested frame recursed to a stack-overflow `abort()` — uncatchable,
//! killing the whole process (and every connection on it) from an unauthenticated
//! client. The fix depth-checks the raw bytes before decoding.
//!
//! F3: inbound WS frames had no size cap (tungstenite default 64 MiB). The fix
//! caps inbound message/frame size on the upgrade.
//!
//! These tests boot a real server IN-PROCESS. If the F1 fix regressed, the deep
//! frame would `SIGABRT` the test runner itself — so "the test completes and the
//! server stays responsive" *is* the proof the crash is fixed. Each test asserts
//! the process survives a hostile frame and unrelated connections are unaffected.

use std::time::Duration;

use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use topgun_server::network::{NetworkConfig, NetworkModule};

/// Boots a minimal server on an OS-assigned port (no JWT → unauthenticated).
/// Returns `(port, shutdown_tx, serve_handle)`.
async fn start_server() -> (
    u16,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let mut module = NetworkModule::new(NetworkConfig::default());
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

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(port: u16) -> Ws {
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (ws, _) = connect_async(&url)
        .await
        .expect("WebSocket connect should succeed");
    ws
}

/// `depth`-deep nested 1-element arrays (`0x91`) terminated by nil (`0xc0`) — the
/// audit repro payload. A `depth + 1`-byte frame that forces `depth`-deep decode
/// recursion. At 200k deep it is far over the 128-level cap yet well under the
/// 2 MB size cap, so it reaches (and must be rejected by) the decoder.
fn nested_frame(depth: usize) -> Vec<u8> {
    let mut buf = vec![0x91u8; depth];
    buf.push(0xc0);
    buf
}

/// Confirms the server process is alive and responsive via the health endpoint.
async fn assert_server_alive(port: u16) {
    let resp = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .expect("server must still answer /health (process survived)");
    assert!(
        resp.status().is_success(),
        "/health returned {}",
        resp.status()
    );
}

/// F1: a deeply-nested pre-auth frame must NOT crash the server. The connection
/// that sent it is dropped harmlessly; every other connection and the process
/// itself survive.
#[tokio::test(flavor = "multi_thread")]
async fn deeply_nested_frame_does_not_crash_server() {
    let (port, shutdown_tx, _serve) = start_server().await;

    // A bystander connection that must survive the attack untouched.
    let mut bystander = connect(port).await;

    // The attacker sends one deeply-nested frame. Pre-fix this aborts the whole
    // process (SIGABRT) — which would also kill this test runner.
    let mut attacker = connect(port).await;
    attacker
        .send(WsMessage::binary(nested_frame(200_000)))
        .await
        .expect("send of hostile frame should leave the socket usable");

    // Give the server time to read + reject the frame.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Proof 1: the process is alive and serving.
    assert_server_alive(port).await;

    // Proof 2: the bystander connection is untouched — it can still send.
    bystander
        .send(WsMessage::binary(nested_frame(8)))
        .await
        .expect("bystander connection must remain usable after the attack");

    // Proof 3: new connections are still accepted.
    let mut latecomer = connect(port).await;
    latecomer
        .send(WsMessage::binary(nested_frame(8)))
        .await
        .expect("server must still accept new connections after the attack");

    let _ = shutdown_tx.send(());
}

/// Negative control: a SHALLOW nested frame (within the depth cap) is handled
/// gracefully — proving the rejection is depth-driven, not a blanket refusal of
/// all nested input, and the server stays up either way.
#[tokio::test(flavor = "multi_thread")]
async fn shallow_frame_is_handled_gracefully() {
    let (port, shutdown_tx, _serve) = start_server().await;

    let mut conn = connect(port).await;
    // 64-deep is valid MsgPack and under the 128 cap. It is not a valid
    // TopGunMessage, so the server drops it — but without crashing or closing.
    conn.send(WsMessage::binary(nested_frame(64)))
        .await
        .expect("send should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_server_alive(port).await;

    // The connection should still be usable.
    conn.send(WsMessage::binary(nested_frame(8)))
        .await
        .expect("connection should remain usable after a shallow frame");

    let _ = shutdown_tx.send(());
}

/// F3: an oversized frame (> the 2 MB inbound cap) is rejected by the transport;
/// the process survives and other connections are unaffected.
#[tokio::test(flavor = "multi_thread")]
async fn oversized_frame_is_rejected_process_survives() {
    let (port, shutdown_tx, _serve) = start_server().await;

    let mut bystander = connect(port).await;

    let mut attacker = connect(port).await;
    // 3 MB of payload — over the 2 MB cap. The server's transport rejects the
    // frame and closes that connection; it must not buffer it or crash.
    let oversized = vec![0u8; 3 * 1024 * 1024];
    // The send itself may or may not error depending on when the server closes;
    // either way the process must survive, which is what we assert.
    let _ = attacker.send(WsMessage::binary(oversized)).await;

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_server_alive(port).await;

    bystander
        .send(WsMessage::binary(nested_frame(8)))
        .await
        .expect("bystander must survive an oversized frame on another connection");

    let _ = shutdown_tx.send(());
}
