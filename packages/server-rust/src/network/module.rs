//! Network module with deferred startup lifecycle.
//!
//! Implements the deferred startup pattern: `new()` creates resources,
//! `start()` binds the TCP listener, and `serve()` starts accepting
//! connections. This separation allows the rest of the application to
//! configure shared state (e.g., cluster, storage) between `start()` and
//! `serve()`.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use axum::routing::{delete, get, post};
use axum::Router;
use tokio::net::TcpListener;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::PeerIpKeyExtractor;
use tower_governor::GovernorLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};
use utoipa::OpenApi;

use super::config::NetworkConfig;
use super::connection::{ConnectionRegistry, OutboundMessage};
use super::handlers::admin::{
    cluster_status, create_policy, delete_policy, get_settings, list_maps, list_policies, login,
    server_status, update_settings,
};
use super::handlers::auth_provider::{AuthProvider, AuthProviderConfig, HmacProvider, JwksProvider, OidcProvider};
use super::handlers::{
    health_handler, http_sync_handler, liveness_handler, metrics_handler, readiness_handler,
    token_exchange_handler, ws_upgrade_handler, AppState,
};
use super::middleware::build_http_layers;
use super::openapi::AdminApiDoc;
use super::shutdown::ShutdownController;
use crate::cluster::state::ClusterState;
use crate::service::config::ServerConfig;
use crate::service::middleware::ObservabilityHandle;
use crate::service::policy::PolicyStore;
use crate::storage::factory::RecordStoreFactory;

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
    observability: Option<Arc<ObservabilityHandle>>,
    cluster_state: Option<Arc<ClusterState>>,
    store_factory: Option<Arc<RecordStoreFactory>>,
    server_config: Option<Arc<ArcSwap<ServerConfig>>>,
    policy_store: Option<Arc<dyn PolicyStore>>,
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
            observability: None,
            cluster_state: None,
            store_factory: None,
            server_config: None,
            policy_store: None,
        }
    }

    /// Configures the observability handle for the `/metrics` endpoint.
    ///
    /// Call this after `new()` and before `start()` to enable Prometheus metrics
    /// scraping at `GET /metrics`.  When not called, the `/metrics` endpoint
    /// returns an empty 200 response (graceful degradation).
    pub fn set_observability(&mut self, handle: Arc<ObservabilityHandle>) {
        self.observability = Some(handle);
    }

    /// Configures the cluster state for the admin cluster status endpoint.
    pub fn set_cluster_state(&mut self, state: Arc<ClusterState>) {
        self.cluster_state = Some(state);
    }

    /// Configures the record store factory for the admin maps endpoint.
    pub fn set_store_factory(&mut self, factory: Arc<RecordStoreFactory>) {
        self.store_factory = Some(factory);
    }

    /// Configures the hot-reloadable server config for admin settings endpoints.
    pub fn set_server_config(&mut self, config: Arc<ArcSwap<ServerConfig>>) {
        self.server_config = Some(config);
    }

    /// Configures the policy store for permission policy admin endpoints.
    pub fn set_policy_store(&mut self, store: Arc<dyn PolicyStore>) {
        self.policy_store = Some(store);
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
    /// - `GET /metrics` -- Prometheus metrics endpoint
    pub fn build_router(&self) -> Router {
        build_app(
            self.config.clone(),
            Arc::clone(&self.registry),
            Arc::clone(&self.shutdown),
            AppServices {
                observability: self.observability.clone(),
                cluster_state: self.cluster_state.clone(),
                store_factory: self.store_factory.clone(),
                server_config: self.server_config.clone(),
                policy_store: self.policy_store.clone(),
            },
        )
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
        let mut config = self.config;

        // Extract TLS config before consuming config into the router,
        // since build_app takes ownership of config.
        let tls = config.tls.take();
        let observability = self.observability.clone();

        let router = build_app(
            config,
            Arc::clone(&registry),
            Arc::clone(&shutdown_ctrl),
            AppServices {
                observability,
                cluster_state: self.cluster_state,
                store_factory: self.store_factory,
                server_config: self.server_config,
                policy_store: self.policy_store,
            },
        );

        // Transition to Ready so readiness probes pass.
        shutdown_ctrl.set_ready();

        if let Some(ref tls_config) = tls {
            serve_tls(listener, router, tls_config, registry, shutdown_ctrl, shutdown).await
        } else {
            serve_plain(listener, router, registry, shutdown_ctrl, shutdown).await
        }
    }
}

/// Bundle of optional services passed to `build_app`.
///
/// Grouping optional fields avoids exceeding the clippy 7-argument limit and
/// makes it clear which parameters are required vs. optional at construction.
struct AppServices {
    observability: Option<Arc<ObservabilityHandle>>,
    cluster_state: Option<Arc<ClusterState>>,
    store_factory: Option<Arc<RecordStoreFactory>>,
    server_config: Option<Arc<ArcSwap<ServerConfig>>>,
    policy_store: Option<Arc<dyn PolicyStore>>,
}

/// Builds the complete application router with all routes and middleware.
///
/// Takes ownership of `config` to avoid an extra clone: `build_http_layers`
/// borrows it first, then it is moved into `Arc` for the `AppState`.
fn build_app(
    config: NetworkConfig,
    registry: Arc<ConnectionRegistry>,
    shutdown: Arc<ShutdownController>,
    services: AppServices,
) -> Router {
    let AppServices {
        observability,
        cluster_state,
        store_factory,
        server_config,
        policy_store,
    } = services;
    let layers = build_http_layers(&config);

    // Load JWT secret from environment so the server can authenticate tokens
    // without requiring secret injection through application code paths.
    let jwt_secret = std::env::var("JWT_SECRET")
        .ok()
        .filter(|s| !s.is_empty());

    // Refuse to start when auth is required but no secret is configured.
    // Only enforced when server_config is Some (production paths). Unit tests
    // and the load harness construct AppState directly with server_config: None
    // and bypass this check intentionally.
    if let Some(ref sc) = server_config {
        let cfg = sc.load();
        assert!(
            !(cfg.security.require_auth && jwt_secret.is_none()),
            "JWT_SECRET environment variable is required when require_auth is true"
        );
    }

    // Warn operators who have JWT auth enabled but no TLS configured: tokens
    // will be transmitted in plaintext over ws://.
    if jwt_secret.is_some() && config.tls.is_none() {
        warn!(
            "JWT authentication is enabled but TLS is not configured. \
             Credentials will be sent in plaintext over ws://. \
             Configure TLS for production deployments."
        );
    }

    // Extract rate limit config before config is moved into Arc<AppState>.
    let rate_limit_per_ip = config.rate_limit_per_ip;
    let rate_limit_burst = config.rate_limit_burst;

    // Build a single shared HTTP client for all JWKS/OIDC providers so that
    // connection pooling is shared rather than fragmented across providers.
    let http_client = reqwest::Client::new();

    // Construct provider instances from config. HmacProvider needs no HTTP
    // client; JwksProvider and OidcProvider share the client.
    let auth_providers: Vec<Arc<dyn AuthProvider>> = config
        .auth_providers
        .iter()
        .map(|cfg| -> Arc<dyn AuthProvider> {
            match cfg {
                AuthProviderConfig::Jwks { name, jwks_url, issuer, audience, claims } => Arc::new(
                    JwksProvider::new(
                        name.clone(),
                        jwks_url.clone(),
                        issuer.clone(),
                        audience.clone(),
                        claims.clone(),
                        http_client.clone(),
                    ),
                ),
                AuthProviderConfig::Oidc { name, issuer_url, audience, claims } => Arc::new(
                    OidcProvider::new(
                        name.clone(),
                        issuer_url.clone(),
                        audience.clone(),
                        claims.clone(),
                        http_client.clone(),
                    ),
                ),
                AuthProviderConfig::Hmac { name, secret, issuer, claims } => Arc::new(
                    HmacProvider::new(
                        name.clone(),
                        secret.clone(),
                        issuer.clone(),
                        claims.clone(),
                    ),
                ),
            }
        })
        .collect();

    let state = AppState {
        registry,
        shutdown,
        config: Arc::new(config),
        start_time: Instant::now(),
        observability,
        operation_service: None,
        dispatcher: None,
        jwt_secret,
        cluster_state,
        store_factory,
        server_config,
        policy_store,
        auth_providers: Arc::new(auth_providers),
    };

    // Build a per-IP rate limiter for admin and login endpoints.
    // Replenishment interval derived from rate_limit_per_ip: for 100 req/s the
    // governor refills 1 token every 10 ms. burst_size controls the initial
    // burst window. /ws and /sync are excluded because those have
    // operation-level load shedding instead of HTTP-layer rate limiting.
    let replenish_interval_ms = (1000 / u64::from(rate_limit_per_ip).max(1)).max(1);
    let governor_config = GovernorConfigBuilder::default()
        .key_extractor(PeerIpKeyExtractor)
        .per_millisecond(replenish_interval_ms)
        .burst_size(rate_limit_burst)
        .finish()
        .expect("GovernorConfig should build with non-zero burst and period");

    let governor_layer = GovernorLayer {
        config: Arc::new(governor_config),
    };

    // Swagger UI served at /api/docs
    let swagger_ui = utoipa_swagger_ui::SwaggerUi::new("/api/docs")
        .url("/api/openapi.json", AdminApiDoc::openapi());

    // Static SPA serving for admin dashboard
    let admin_spa_dir = std::env::var("TOPGUN_ADMIN_DIR")
        .unwrap_or_else(|_| "./admin-dashboard/dist".to_string());
    let index_html = format!("{admin_spa_dir}/index.html");
    let serve_dir = ServeDir::new(&admin_spa_dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index_html));

    // Rate-limited routes: admin API and login are brute-force targets and
    // get per-IP throttling. All other routes (health, ws, sync, metrics,
    // docs, status, admin SPA) bypass the governor.
    let rate_limited_routes = Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/token", post(token_exchange_handler))
        .route("/api/admin/cluster/status", get(cluster_status))
        .route("/api/admin/maps", get(list_maps))
        .route("/api/admin/settings", get(get_settings).put(update_settings))
        .route(
            "/api/admin/policies",
            get(list_policies).post(create_policy),
        )
        .route("/api/admin/policies/{id}", delete(delete_policy))
        .route_layer(governor_layer);

    Router::new()
        // Health probes -- never rate-limited (Kubernetes liveness/readiness)
        .route("/health", get(health_handler))
        .route("/health/live", get(liveness_handler))
        .route("/health/ready", get(readiness_handler))
        // Real-time transport -- operation-level load shedding, not HTTP rate limiting
        .route("/ws", get(ws_upgrade_handler))
        .route("/sync", post(http_sync_handler))
        // Metrics and status -- internal observability endpoints
        .route("/metrics", get(metrics_handler))
        .route("/api/status", get(server_status))
        // Rate-limited admin and login routes
        .merge(rate_limited_routes)
        // Swagger UI serves both the JSON spec at /api/openapi.json and the UI at /api/docs
        .merge(swagger_ui)
        // Static SPA for admin dashboard -- served as static files, not rate-limited
        .nest_service("/admin", serve_dir)
        .layer(layers)
        .with_state(state)
}

/// Serves plain HTTP/WS connections using axum's built-in server.
///
/// Uses `into_make_service_with_connect_info` so that the peer `SocketAddr` is
/// injected into each request's extensions. This is required by
/// `PeerIpKeyExtractor` to identify which IP to rate-limit.
async fn serve_plain(
    listener: TcpListener,
    router: Router,
    registry: Arc<ConnectionRegistry>,
    shutdown_ctrl: Arc<ShutdownController>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    info!("Serving plain HTTP/WS connections");

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
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
    use crate::network::HealthState;

    // ── Test helper ───────────────────────────────────────────────────

    /// Starts a server on an OS-assigned port and returns the port,
    /// shared registry, shared shutdown controller, a oneshot sender
    /// that triggers graceful shutdown when sent or dropped, and a
    /// `JoinHandle` for the serve task so callers can verify completion.
    async fn start_server() -> (
        u16,
        Arc<ConnectionRegistry>,
        Arc<ShutdownController>,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let mut module = NetworkModule::new(NetworkConfig::default());
        let registry = module.registry();
        let shutdown_ctrl = module.shutdown_controller();
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

        // Give the server a moment to transition to Ready.
        tokio::time::sleep(Duration::from_millis(50)).await;

        (port, registry, shutdown_ctrl, shutdown_tx, serve_handle)
    }

    // ── Unit tests ────────────────────────────────────────────────────

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

    // ── Integration tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn health_endpoint_responds_with_ready() {
        let (port, _registry, _shutdown_ctrl, shutdown_tx, _handle) = start_server().await;

        let resp = reqwest::get(format!("http://127.0.0.1:{port}/health"))
            .await
            .expect("health request should succeed");

        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.expect("body should be JSON");
        assert_eq!(body["state"], "ready");

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn liveness_and_readiness_endpoints() {
        let (port, _registry, _shutdown_ctrl, shutdown_tx, _handle) = start_server().await;

        let live_resp = reqwest::get(format!("http://127.0.0.1:{port}/health/live"))
            .await
            .expect("liveness request should succeed");
        assert_eq!(live_resp.status(), 200);

        let ready_resp = reqwest::get(format!("http://127.0.0.1:{port}/health/ready"))
            .await
            .expect("readiness request should succeed");
        assert_eq!(ready_resp.status(), 200);

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn websocket_upgrade_and_registry_tracking() {
        let (port, registry, _shutdown_ctrl, shutdown_tx, _handle) = start_server().await;

        assert_eq!(registry.count(), 0, "no connections initially");

        let (ws_stream, _response) = tokio_tungstenite::connect_async(
            format!("ws://127.0.0.1:{port}/ws"),
        )
        .await
        .expect("WS connect should succeed");

        // Wait for the server to register the connection.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(registry.count(), 1, "one WS connection registered");

        // Drop the WS stream to trigger disconnect.
        drop(ws_stream);

        // Poll until the server deregisters the connection.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if registry.count() == 0 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "registry.count() did not reach 0 within 2s, current: {}",
                registry.count()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn post_sync_returns_msgpack() {
        let (port, _registry, _shutdown_ctrl, shutdown_tx, _handle) = start_server().await;

        let client = reqwest::Client::new();
        // Empty body is treated as an empty HttpSyncRequest; handler returns 200
        // with a MsgPack-encoded HttpSyncResponse.
        let resp = client
            .post(format!("http://127.0.0.1:{port}/sync"))
            .body(vec![])
            .send()
            .await
            .expect("POST /sync should succeed");

        assert_eq!(resp.status(), 200);

        let content_type = resp
            .headers()
            .get("content-type")
            .expect("content-type header should be present")
            .to_str()
            .expect("content-type should be valid UTF-8");
        assert_eq!(content_type, "application/msgpack");

        let body = resp.bytes().await.expect("body read should succeed");
        let decoded: topgun_core::messages::HttpSyncResponse =
            rmp_serde::from_slice(&body).expect("response body should be valid MsgPack HttpSyncResponse");
        // server_hlc.millis must be populated with a positive wall-clock value.
        assert!(decoded.server_hlc.millis > 0);

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn post_sync_malformed_body_returns_400() {
        let (port, _registry, _shutdown_ctrl, shutdown_tx, _handle) = start_server().await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{port}/sync"))
            .body(vec![0xFF_u8, 0xFF_u8])
            .send()
            .await
            .expect("POST /sync request should complete");

        assert_eq!(resp.status(), 400);

        drop(shutdown_tx);
    }

    #[tokio::test]
    async fn graceful_shutdown_drains_and_stops() {
        let (port, _registry, shutdown_ctrl, shutdown_tx, serve_handle) = start_server().await;

        assert_eq!(shutdown_ctrl.health_state(), HealthState::Ready);

        // Verify the server is accepting requests.
        let resp = reqwest::get(format!("http://127.0.0.1:{port}/health"))
            .await
            .expect("health request should succeed before shutdown");
        assert_eq!(resp.status(), 200);

        // Trigger shutdown.
        drop(shutdown_tx);

        // Verify that the serve task completes within a reasonable timeout.
        tokio::time::timeout(Duration::from_secs(5), serve_handle)
            .await
            .expect("serve task should complete within 5s")
            .expect("serve task should not panic");

        // After serve completes, health state should be Draining or Stopped.
        let state = shutdown_ctrl.health_state();
        assert!(
            state == HealthState::Draining || state == HealthState::Stopped,
            "expected Draining or Stopped after serve completes, got {state:?}"
        );
    }

    #[tokio::test]
    async fn request_id_header_is_present_in_response() {
        let (port, _registry, _shutdown_ctrl, shutdown_tx, _handle) = start_server().await;

        let resp = reqwest::get(format!("http://127.0.0.1:{port}/health"))
            .await
            .expect("health request should succeed");

        let request_id = resp
            .headers()
            .get("x-request-id")
            .expect("X-Request-Id header should be present in response");

        // UUID v4 format: 8-4-4-4-12 hex characters
        let id_str = request_id
            .to_str()
            .expect("X-Request-Id should be valid UTF-8");
        assert!(
            !id_str.is_empty(),
            "X-Request-Id should not be empty"
        );
        assert_eq!(
            id_str.len(),
            36,
            "X-Request-Id should be a UUID (36 chars): {id_str}"
        );

        drop(shutdown_tx);
    }

    /// Starts a server with a custom `NetworkConfig` and returns the same
    /// tuple as `start_server`.
    async fn start_server_with_config(
        config: NetworkConfig,
    ) -> (
        u16,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
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

    /// Verifies that sending more requests than `rate_limit_burst` to a
    /// rate-limited endpoint returns HTTP 429 with a `Retry-After` header.
    /// Endpoints like /health and /sync must never be affected.
    #[tokio::test]
    async fn rate_limit_triggers_429_on_admin_endpoint_after_burst() {
        let config = NetworkConfig {
            // A burst of 2 means the 3rd and subsequent requests are throttled.
            rate_limit_burst: 2,
            ..NetworkConfig::default()
        };
        let (port, shutdown_tx, _handle) = start_server_with_config(config).await;

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/api/admin/cluster/status");

        let mut last_status = reqwest::StatusCode::OK;
        // Send burst+2 requests; at least one must return 429 once the burst is exhausted.
        for _ in 0..(2 + 2) {
            let resp = client
                .get(&url)
                .send()
                .await
                .expect("request should complete");
            last_status = resp.status();
            if last_status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                // Confirm Retry-After header is present on 429 responses.
                assert!(
                    resp.headers().contains_key("retry-after"),
                    "429 response must include Retry-After header"
                );
                break;
            }
        }

        assert_eq!(
            last_status,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "Expected 429 after exhausting burst of 2; last status was {last_status}"
        );

        drop(shutdown_tx);
    }

    /// Verifies that /health is never affected by rate limiting even when
    /// the burst limit is set to a very low value.
    #[tokio::test]
    async fn rate_limit_does_not_apply_to_health_endpoint() {
        let config = NetworkConfig {
            rate_limit_burst: 1,
            ..NetworkConfig::default()
        };
        let (port, shutdown_tx, _handle) = start_server_with_config(config).await;

        let client = reqwest::Client::new();

        // Send many requests to /health; none should be rate-limited.
        for _ in 0..5 {
            let resp = client
                .get(format!("http://127.0.0.1:{port}/health"))
                .send()
                .await
                .expect("health request should succeed");
            assert_eq!(
                resp.status(),
                200,
                "/health must never return 429 regardless of rate limit config"
            );
        }

        drop(shutdown_tx);
    }
}
