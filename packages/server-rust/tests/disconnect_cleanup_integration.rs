//! Integration test: WebSocket disconnect deterministically releases all
//! session-scoped registry state (locks, topic subscriptions, counter
//! subscriptions).
//!
//! Strategy: boot a minimal axum server with real registries wired into
//! `AppState`. Connect a WebSocket client, seed the three registries with
//! the client's `ConnectionId`, drop the client, and verify all three
//! registry entries are cleaned within a bounded poll window.
//!
//! No JWT is configured so the server skips the auth phase and goes
//! straight to Phase 2, which means any close (including a plain TCP
//! drop) hits the normal-close path that calls `release_session_state`.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::network::handlers::AppState;
use topgun_server::service::domain::coordination_lock::LockRegistry;
use topgun_server::service::domain::counter::CounterRegistry;
use topgun_server::service::domain::messaging::TopicRegistry;

// ---------------------------------------------------------------------------
// Server-boot helper
// ---------------------------------------------------------------------------

/// Boot a minimal axum server on an OS-assigned port with the three session
/// registries wired. Returns (port, registry Arc clones, shutdown trigger).
async fn start_server_with_registries() -> (
    u16,
    Arc<ConnectionRegistry>,
    Arc<LockRegistry>,
    Arc<TopicRegistry>,
    Arc<CounterRegistry>,
    tokio::sync::oneshot::Sender<()>,
) {
    let connection_registry = Arc::new(ConnectionRegistry::new());
    let lock_reg = Arc::new(LockRegistry::new());
    let topic_reg = Arc::new(TopicRegistry::new());
    let counter_reg = Arc::new(CounterRegistry::new("test-node".to_string()));

    let state = AppState {
        registry: Arc::clone(&connection_registry),
        lock_registry: Some(Arc::clone(&lock_reg)),
        topic_registry: Some(Arc::clone(&topic_reg)),
        counter_registry: Some(Arc::clone(&counter_reg)),
        // jwt_secret: None — skip auth phase, go straight to Phase 2.
        ..AppState::for_test()
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind should succeed");
    let port = listener.local_addr().expect("local_addr").port();

    let shutdown_ctrl = Arc::clone(&state.shutdown);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let app = axum::Router::new()
        .route(
            "/ws",
            axum::routing::get(topgun_server::network::handlers::ws_upgrade_handler),
        )
        .with_state(state);

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
                shutdown_ctrl.trigger_shutdown();
            })
            .await
            .expect("serve should not fail");
    });

    // Give the server a moment to start.
    tokio::time::sleep(Duration::from_millis(30)).await;

    (
        port,
        connection_registry,
        lock_reg,
        topic_reg,
        counter_reg,
        shutdown_tx,
    )
}

// ---------------------------------------------------------------------------
// Integration test: combined lock + topic + counter cleanup on disconnect
// ---------------------------------------------------------------------------

/// AC5: Dropping a WebSocket that has session state in all three registries
/// deterministically releases that state within the close handler.
///
/// Steps:
/// 1. Boot the server with real registries.
/// 2. Connect a client WebSocket; wait for the server to register it.
/// 3. Seed the three registries with the client's `ConnectionId`.
/// 4. Drop the client connection (no explicit release).
/// 5. Poll (bounded 1 s) until the server processes the close.
/// 6. Verify lock is released, topic subscription removed, counter
///    subscription removed.
#[tokio::test(flavor = "multi_thread")]
async fn disconnect_releases_lock_topic_and_counter_state() {
    let (port, conn_reg, lock_reg, topic_reg, counter_reg, shutdown_tx) =
        start_server_with_registries().await;

    // Connect to the server.
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (ws_stream, _) = connect_async(&url)
        .await
        .expect("WebSocket connect should succeed");

    // Wait for the server to register the connection.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while conn_reg.count() == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not register the connection within 1 s"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Retrieve the ConnectionId assigned by the server.
    let handles = conn_reg.connections();
    assert_eq!(
        handles.len(),
        1,
        "exactly one connection should be registered"
    );
    let conn_id = handles[0].id;

    // Seed the three registries as if this connection holds session state.
    lock_reg
        .try_acquire("L1", conn_id, Some(60_000))
        .expect("lock acquire should succeed");
    topic_reg
        .subscribe("T1", conn_id)
        .expect("topic subscribe should succeed");
    counter_reg.subscribe("C1", conn_id);

    // Verify preconditions: all three registries contain the connection.
    assert_eq!(
        lock_reg.holder("L1"),
        Some(conn_id),
        "L1 should be held by our connection"
    );
    assert!(
        topic_reg.subscribers("T1").contains(&conn_id),
        "T1 should list our connection as a subscriber"
    );
    assert!(
        counter_reg.subscribers("C1").contains(&conn_id),
        "C1 should list our connection as a subscriber"
    );

    // Send an explicit Close frame to trigger the server's close path.
    // An explicit Close frame is the most reliable way to signal disconnect
    // without relying on OS-level TCP teardown timing.
    let (mut write_half, _read_half) = ws_stream.split();
    let _ = write_half.send(WsMessage::Close(None)).await;
    drop(write_half);

    // Poll until the server processes the disconnect (bounded 3 s).
    // The server waits up to 2 s for the outbound task to drain before
    // calling release_session_state, so 3 s gives 1 s of slack.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let lock_released = lock_reg.holder("L1").is_none();
        let topic_cleaned = !topic_reg.subscribers("T1").contains(&conn_id);
        let counter_cleaned = !counter_reg.subscribers("C1").contains(&conn_id);

        if lock_released && topic_cleaned && counter_cleaned {
            break;
        }

        if tokio::time::Instant::now() >= deadline {
            assert!(
                lock_released,
                "lock L1 was not released within 1 s after disconnect"
            );
            assert!(
                topic_cleaned,
                "topic T1 still lists the disconnected conn_id after 1 s"
            );
            assert!(
                counter_cleaned,
                "counter C1 still lists the disconnected conn_id after 1 s"
            );
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Shut down the server cleanly.
    let _ = shutdown_tx.send(());
}
