//! Network module with deferred startup lifecycle.
//!
//! Implements the deferred startup pattern: `new()` creates resources,
//! `start()` binds the TCP listener, and `serve()` starts accepting
//! connections. This separation allows the rest of the application to
//! configure shared state (e.g., cluster, storage) between `start()` and
//! `serve()`.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tracing::{info, warn};

use super::config::NetworkConfig;
use super::connection::{ConnectionRegistry, OutboundMessage};
use super::handlers::{
    health_handler, http_sync_handler, liveness_handler, readiness_handler, ws_upgrade_handler,
    AppState,
};
use super::middleware::build_http_layers;
use super::shutdown::ShutdownController;

/// Manages the full HTTP/WebSocket server lifecycle.
///
/// Follows the deferred startup pattern:
/// 1. `new()` -- allocates shared state (registry, shutdown controller)
/// 2. `start()` -- binds TCP listener to the configured address
/// 3. `serve()` -- begins accepting connections until shutdown is signalled
///
/// The registry and shutdown controller are shared via `Arc` so other
/// modules (cluster, storage) can reference them after construction.
pub struct NetworkModule {
    config: NetworkConfig,
    listener: Option<TcpListener>,
    registry: Arc<ConnectionRegistry>,
    shutdown: Arc<ShutdownController>,
}

impl NetworkModule {
    /// Creates a new network module without binding any port.
    ///
    /// The registry and shutdown controller are allocated immediately so
    /// they can be shared with other modules before the server starts.
    #[must_use]
    pub fn new(config: NetworkConfig) -> Self {
        Self {
            config,
            listener: None,
            registry: Arc::new(ConnectionRegistry::new()),
            shutdown: Arc::new(ShutdownController::new()),
        }
    }

    /// Returns a shared reference to the connection registry.
    ///
    /// Other modules use this to inspect or broadcast to active connections.
    #[must_use]
    pub fn registry(&self) -> Arc<ConnectionRegistry> {
        Arc::clone(&self.registry)
    }

    /// Returns a shared reference to the shutdown controller.
    ///
    /// Other modules use this to check health state or trigger shutdown.
    #[must_use]
    pub fn shutdown_controller(&self) -> Arc<ShutdownController> {
        Arc::clone(&self.shutdown)
    }

    /// Assembles the axum router with all routes and middleware.
    ///
    /// Routes:
    /// - `GET /health` -- detailed health JSON
    /// - `GET /health/live` -- Kubernetes liveness probe
    /// - `GET /health/ready` -- Kubernetes readiness probe
    /// - `GET /ws` -- WebSocket upgrade
    /// - `POST /sync` -- HTTP sync endpoint (`MsgPack`)
    pub fn build_router(&self) -> Router {
        let state = AppState {
            registry: Arc::clone(&self.registry),
            shutdown: Arc::clone(&self.shutdown),
            config: Arc::new(self.config.clone()),
            start_time: Instant::now(),
        };

        let layers = build_http_layers(&self.config);

        Router::new()
            .route("/health", get(health_handler))
            .route("/health/live", get(liveness_handler))
            .route("/health/ready", get(readiness_handler))
            .route("/ws", get(ws_upgrade_handler))
            .route("/sync", post(http_sync_handler))
            .layer(layers)
            .with_state(state)
    }

    /// Binds the TCP listener to the configured host and port.
    ///
    /// Returns the actual bound port, which may differ from the configured
    /// port when port 0 is used (OS-assigned ephemeral port).
    ///
    /// # Errors
    ///
    /// Returns an error if the address cannot be bound (e.g., port in use).
    pub async fn start(&mut self) -> anyhow::Result<u16> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let listener = TcpListener::bind(&addr).await?;
        let port = listener.local_addr()?.port();

        info!("TCP listener bound to {}:{}", self.config.host, port);

        self.listener = Some(listener);
        Ok(port)
    }

    /// Starts serving connections until the shutdown signal fires.
    ///
    /// Consumes `self` because the listener is moved into the server.
    /// Panics if `start()` was not called first.
    ///
    /// After the shutdown signal:
    /// 1. Health state transitions to Draining
    /// 2. All connections receive a Close frame
    /// 3. Waits up to 30 seconds for in-flight requests to complete
    /// 4. Health state transitions to Stopped
    ///
    /// # Errors
    ///
    /// Returns an error if the server encounters a fatal I/O error.
    ///
    /// # Panics
    ///
    /// Panics if `start()` was not called before `serve()`.
    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        let listener = self
            .listener
            .expect("start() must be called before serve()");
        let registry = self.registry;
        let shutdown_ctrl = self.shutdown;
        let config = self.config;

        // Build the router after extracting all fields from self to avoid
        // partial move issues.
        let state = AppState {
            registry: Arc::clone(&registry),
            shutdown: Arc::clone(&shutdown_ctrl),
            config: Arc::new(config.clone()),
            start_time: Instant::now(),
        };

        let layers = build_http_layers(&config);

        let router = Router::new()
            .route("/health", get(health_handler))
            .route("/health/live", get(liveness_handler))
            .route("/health/ready", get(readiness_handler))
            .route("/ws", get(ws_upgrade_handler))
            .route("/sync", post(http_sync_handler))
            .layer(layers)
            .with_state(state);

        // Transition to Ready so readiness probes pass.
        shutdown_ctrl.set_ready();

        if let Some(ref tls_config) = config.tls {
            serve_tls(listener, router, tls_config, registry, shutdown_ctrl, shutdown).await
        } else {
            serve_plain(listener, router, registry, shutdown_ctrl, shutdown).await
        }
    }
}

/// Serves plain HTTP/WS connections using axum's built-in server.
async fn serve_plain(
    listener: TcpListener,
    router: Router,
    registry: Arc<ConnectionRegistry>,
    shutdown_ctrl: Arc<ShutdownController>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    info!("Serving plain HTTP/WS connections");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;

    drain_connections(registry, shutdown_ctrl).await;
    Ok(())
}

/// Serves TLS connections using `axum-server` with rustls.
///
/// Reuses the pre-bound TCP listener by converting it to a `std::net::TcpListener`.
async fn serve_tls(
    listener: TcpListener,
    router: Router,
    tls_config: &super::config::TlsConfig,
    registry: Arc<ConnectionRegistry>,
    shutdown_ctrl: Arc<ShutdownController>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    use axum_server::tls_rustls::RustlsConfig;

    let rustls_config = RustlsConfig::from_pem_file(&tls_config.cert_path, &tls_config.key_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load TLS certificates: {e}"))?;

    let addr = listener.local_addr()?;
    let std_listener = listener.into_std()?;
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();

    // Spawn a task that waits for the shutdown signal and triggers graceful
    // shutdown on the axum-server handle.
    tokio::spawn(async move {
        shutdown.await;
        shutdown_handle.graceful_shutdown(None);
    });

    info!("Serving TLS connections on {}", addr);

    axum_server::from_tcp_rustls(std_listener, rustls_config)
        .handle(handle)
        .serve(router.into_make_service())
        .await?;

    drain_connections(registry, shutdown_ctrl).await;
    Ok(())
}

/// Drains all connections and transitions to Stopped state.
///
/// Sends a Close frame to every active connection, then waits for
/// in-flight requests to complete (up to 30 seconds).
async fn drain_connections(
    registry: Arc<ConnectionRegistry>,
    shutdown_ctrl: Arc<ShutdownController>,
) {
    shutdown_ctrl.trigger_shutdown();

    let handles = registry.drain_all();
    let count = handles.len();
    if count > 0 {
        info!("Draining {} connections", count);
        for handle in &handles {
            let _ = handle.try_send(OutboundMessage::Close(Some(
                "server shutting down".to_string(),
            )));
        }
    }

    let drained = shutdown_ctrl.wait_for_drain(Duration::from_secs(30)).await;
    if drained {
        info!("All connections drained successfully");
    } else {
        warn!("Drain timeout expired with in-flight requests remaining");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_module_without_binding() {
        let module = NetworkModule::new(NetworkConfig::default());
        assert!(module.listener.is_none());
    }

    #[test]
    fn registry_returns_shared_arc() {
        let module = NetworkModule::new(NetworkConfig::default());
        let r1 = module.registry();
        let r2 = module.registry();
        assert!(Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn shutdown_controller_returns_shared_arc() {
        let module = NetworkModule::new(NetworkConfig::default());
        let s1 = module.shutdown_controller();
        let s2 = module.shutdown_controller();
        assert!(Arc::ptr_eq(&s1, &s2));
    }

    #[test]
    fn build_router_creates_router() {
        let module = NetworkModule::new(NetworkConfig::default());
        let _router = module.build_router();
    }

    #[tokio::test]
    async fn start_binds_to_os_assigned_port() {
        let mut module = NetworkModule::new(NetworkConfig::default());
        let port = module.start().await.expect("start should succeed");
        assert!(port > 0, "OS-assigned port should be > 0");
        assert!(module.listener.is_some());
    }

    #[tokio::test]
    #[should_panic(expected = "start() must be called before serve()")]
    async fn serve_panics_without_start() {
        let module = NetworkModule::new(NetworkConfig::default());
        let _ = module.serve(std::future::pending::<()>()).await;
    }
}
