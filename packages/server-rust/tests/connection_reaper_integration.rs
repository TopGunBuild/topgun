//! Integration test: the connection reaper evicts idle/half-open and
//! never-authenticated WebSocket connections, and the read loop runs its full
//! cleanup path when reaped, so connection slots and per-connection resources
//! return to zero.
//!
//! Strategy mirrors `disconnect_cleanup_integration.rs`: boot a minimal axum
//! server with real registries wired into `AppState`, connect real WebSocket
//! clients, and observe registry state. The difference is that here the client
//! never disconnects — the *server-side reaper* is what closes the connection.
//!
//! Coverage:
//! - idle connection is reaped after the idle timeout, and its seeded session
//!   state (lock/topic/counter) is released → resources freed (AC: reaped
//!   connection cleans up all per-connection resources, per SPEC-318).
//! - negative control: with a long idle timeout the same idle connection
//!   survives the observation window (reaper does not kill healthy connections).
//! - negative control: with no reaper running, an idle connection hangs forever
//!   (the leak this fix closes).
//! - slowloris: with auth required and a short auth deadline, a client that
//!   connects but never authenticates is closed by the Phase-1 deadline.
//! - churn: N connect-then-idle connections all return the connection count to
//!   zero after the timeout.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use topgun_server::network::config::{ConnectionConfig, NetworkConfig};
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::network::handlers::AppState;
use topgun_server::network::reaper::{spawn_connection_reaper, ReaperConfig};
use topgun_server::service::domain::coordination_lock::LockRegistry;
use topgun_server::service::domain::counter::CounterRegistry;
use topgun_server::service::domain::messaging::TopicRegistry;

/// Knobs for booting a test server.
struct BootOpts {
    /// When set, the server requires JWT auth (Phase 1 runs) with this secret.
    jwt_secret: Option<String>,
    /// Per-connection auth deadline (Phase-1 slowloris bound).
    auth_timeout: Duration,
    /// When `Some`, spawn the reaper with these tunables.
    reaper: Option<ReaperConfig>,
}

struct Booted {
    port: u16,
    conn_reg: Arc<ConnectionRegistry>,
    lock_reg: Arc<LockRegistry>,
    topic_reg: Arc<TopicRegistry>,
    counter_reg: Arc<CounterRegistry>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    reaper_handle: Option<tokio::task::JoinHandle<()>>,
}

async fn boot(opts: BootOpts) -> Booted {
    let connection_registry = Arc::new(ConnectionRegistry::new());
    let lock_reg = Arc::new(LockRegistry::new());
    let topic_reg = Arc::new(TopicRegistry::new());
    let counter_reg = Arc::new(CounterRegistry::new("test-node".to_string()));

    let net_config = NetworkConfig {
        connection: ConnectionConfig {
            auth_timeout: opts.auth_timeout,
            ..ConnectionConfig::default()
        },
        ..NetworkConfig::default()
    };

    let state = AppState {
        registry: Arc::clone(&connection_registry),
        config: Arc::new(net_config),
        jwt_secret: opts.jwt_secret,
        lock_registry: Some(Arc::clone(&lock_reg)),
        topic_registry: Some(Arc::clone(&topic_reg)),
        counter_registry: Some(Arc::clone(&counter_reg)),
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

    let reaper_handle = opts
        .reaper
        .map(|cfg| spawn_connection_reaper(Arc::clone(&connection_registry), cfg));

    // Give the server a moment to start.
    tokio::time::sleep(Duration::from_millis(30)).await;

    Booted {
        port,
        conn_reg: connection_registry,
        lock_reg,
        topic_reg,
        counter_reg,
        shutdown_tx,
        reaper_handle,
    }
}

/// Connect a WebSocket client and wait until the server registers it, returning
/// the live stream (which the caller must hold to keep the connection open).
async fn connect_and_register(
    port: u16,
    conn_reg: &ConnectionRegistry,
    expected_count: usize,
) -> impl StreamExt {
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (ws_stream, _) = connect_async(&url)
        .await
        .expect("WebSocket connect should succeed");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while conn_reg.count() < expected_count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not register connection(s) within 2 s (have {}, want {})",
            conn_reg.count(),
            expected_count
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    ws_stream
}

async fn poll_until<F: Fn() -> bool>(cond: F, within: Duration, msg: &str) {
    let deadline = tokio::time::Instant::now() + within;
    while !cond() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not met within {within:?}: {msg}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1: idle connection reaped + seeded session state released
// ---------------------------------------------------------------------------

/// An idle connection (no app traffic, no heartbeat) is reaped once it exceeds
/// the idle timeout, and the reaper-triggered teardown releases all seeded
/// lock/topic/counter state and removes the connection from the registry.
#[tokio::test(flavor = "multi_thread")]
async fn idle_connection_reaped_and_resources_freed() {
    // No JWT → connections reach steady state immediately (handshake_complete),
    // so the idle (heartbeat) timeout governs. Short timeouts for a fast test.
    let booted = boot(BootOpts {
        jwt_secret: None,
        auth_timeout: Duration::from_secs(30),
        reaper: Some(ReaperConfig {
            idle_timeout: Duration::from_millis(250),
            auth_timeout: Duration::from_millis(250),
            interval: Duration::from_millis(40),
        }),
    })
    .await;

    let _ws = connect_and_register(booted.port, &booted.conn_reg, 1).await;

    let conn_id = booted.conn_reg.connections()[0].id;

    // Seed session state as if the connection holds locks/topics/counters.
    booted
        .lock_reg
        .try_acquire("L1", conn_id, Some(60_000))
        .expect("lock acquire");
    booted
        .topic_reg
        .subscribe("T1", conn_id)
        .expect("subscribe");
    booted.counter_reg.subscribe("C1", conn_id);

    // The reaper should evict the idle connection and its cleanup should remove
    // it from the registry AND release all seeded state.
    poll_until(
        || booted.conn_reg.count() == 0,
        Duration::from_secs(5),
        "idle connection should be reaped from the registry",
    )
    .await;

    poll_until(
        || {
            booted.lock_reg.holder("L1").is_none()
                && !booted.topic_reg.subscribers("T1").contains(&conn_id)
                && !booted.counter_reg.subscribers("C1").contains(&conn_id)
        },
        Duration::from_secs(3),
        "reaped connection must release lock/topic/counter state",
    )
    .await;

    if let Some(h) = booted.reaper_handle {
        h.abort();
    }
    let _ = booted.shutdown_tx.send(());
}

// ---------------------------------------------------------------------------
// Test 2: negative control — healthy connection survives a long idle timeout
// ---------------------------------------------------------------------------

/// With a long idle timeout, the reaper running on its normal cadence does NOT
/// reap a connection that is merely young. Guards against the reaper killing
/// healthy connections.
#[tokio::test(flavor = "multi_thread")]
async fn reaper_does_not_kill_young_connection() {
    let booted = boot(BootOpts {
        jwt_secret: None,
        auth_timeout: Duration::from_secs(30),
        reaper: Some(ReaperConfig {
            idle_timeout: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(30),
            interval: Duration::from_millis(40),
        }),
    })
    .await;

    let _ws = connect_and_register(booted.port, &booted.conn_reg, 1).await;

    // Let several reaper ticks elapse; the connection must persist.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        booted.conn_reg.count(),
        1,
        "young connection must survive multiple reaper ticks"
    );

    if let Some(h) = booted.reaper_handle {
        h.abort();
    }
    let _ = booted.shutdown_tx.send(());
}

// ---------------------------------------------------------------------------
// Test 3: negative control — without a reaper, an idle connection hangs
// ---------------------------------------------------------------------------

/// Without the reaper, an idle connection is never closed server-side — this is
/// the leak the fix addresses. Demonstrates the reaper is load-bearing.
#[tokio::test(flavor = "multi_thread")]
async fn without_reaper_idle_connection_hangs() {
    let booted = boot(BootOpts {
        jwt_secret: None,
        auth_timeout: Duration::from_secs(30),
        reaper: None, // no reaper
    })
    .await;

    let _ws = connect_and_register(booted.port, &booted.conn_reg, 1).await;

    // Even after a generous wait, the idle connection is still registered.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        booted.conn_reg.count(),
        1,
        "without a reaper, an idle connection must hang (leak the slot)"
    );

    let _ = booted.shutdown_tx.send(());
}

// ---------------------------------------------------------------------------
// Test 4: slowloris — never-authenticated connection closed by auth deadline
// ---------------------------------------------------------------------------

/// With auth required and a short auth deadline, a client that connects but
/// never sends AUTH is closed by the Phase-1 deadline (independent of the
/// reaper), returning the connection count to zero.
#[tokio::test(flavor = "multi_thread")]
async fn slowloris_closed_by_auth_deadline() {
    let booted = boot(BootOpts {
        jwt_secret: Some("test-secret".to_string()),
        auth_timeout: Duration::from_millis(250),
        reaper: None, // prove the per-connection Phase-1 deadline alone suffices
    })
    .await;

    // Connect but never send an AUTH message. The server's Phase-1 deadline
    // should close it.
    let _ws = connect_and_register(booted.port, &booted.conn_reg, 1).await;

    poll_until(
        || booted.conn_reg.count() == 0,
        Duration::from_secs(5),
        "never-authenticated connection should be closed by the auth deadline",
    )
    .await;

    let _ = booted.shutdown_tx.send(());
}

// ---------------------------------------------------------------------------
// Test 5: churn — N connect-then-idle connections all return count to zero
// ---------------------------------------------------------------------------

/// A burst of connect-then-idle connections is fully reaped: the registry count
/// returns to zero after the idle timeout. This is the mini soak/churn signal —
/// no monotonic connection growth.
#[tokio::test(flavor = "multi_thread")]
async fn churn_idle_connections_return_count_to_zero() {
    const N: usize = 20;

    let booted = boot(BootOpts {
        jwt_secret: None,
        auth_timeout: Duration::from_secs(30),
        reaper: Some(ReaperConfig {
            idle_timeout: Duration::from_millis(250),
            auth_timeout: Duration::from_millis(250),
            interval: Duration::from_millis(40),
        }),
    })
    .await;

    // Hold the streams so the clients don't close from the client side; the
    // server-side reaper is what must close them.
    let mut streams = Vec::new();
    for i in 1..=N {
        let ws = connect_and_register(booted.port, &booted.conn_reg, i).await;
        streams.push(ws);
    }
    assert_eq!(booted.conn_reg.count(), N, "all N connections registered");

    poll_until(
        || booted.conn_reg.count() == 0,
        Duration::from_secs(6),
        "all idle connections should be reaped back to zero",
    )
    .await;

    if let Some(h) = booted.reaper_handle {
        h.abort();
    }
    let _ = booted.shutdown_tx.send(());
    drop(streams);
}
