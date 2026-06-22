//! Network configuration types for the `TopGun` server.

use std::path::PathBuf;
use std::time::Duration;

use crate::network::handlers::auth_provider::AuthProviderConfig;

/// Top-level network configuration for the server.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Bind address for the server.
    pub host: String,
    /// Port to listen on. 0 means OS-assigned.
    pub port: u16,
    /// Optional TLS configuration.
    pub tls: Option<TlsConfig>,
    /// Per-connection settings.
    pub connection: ConnectionConfig,
    /// Allowed CORS origins. An empty list rejects all cross-origin requests.
    pub cors_origins: Vec<String>,
    /// Maximum age for CORS preflight cache (Access-Control-Max-Age header).
    pub cors_max_age: Duration,
    /// Whether to include `Access-Control-Allow-Credentials: true` in CORS
    /// responses. Must not be combined with wildcard origins (CORS spec
    /// violation).
    pub cors_allow_credentials: bool,
    /// Maximum allowed request body size in bytes. Bodies exceeding this
    /// limit are rejected with HTTP 413 before deserialization.
    pub max_body_size: usize,
    /// Maximum time to wait for a request to complete.
    pub request_timeout: Duration,
    /// Clock skew tolerance in seconds for JWT `exp` validation.
    ///
    /// Tokens expired within this window are still accepted to handle clock
    /// drift between clients and the server. Defaults to 60 seconds.
    pub jwt_clock_skew_secs: u64,
    /// Maximum sustained requests per second per IP address for rate-limited
    /// endpoints (admin API and login). Defaults to 100.
    pub rate_limit_per_ip: u32,
    /// Burst allowance above the sustained rate. The governor allows up to
    /// `rate_limit_burst` requests in a short window before throttling.
    /// Defaults to 50.
    pub rate_limit_burst: u32,
    /// Whether to trust reverse-proxy forwarding headers (rightmost
    /// `X-Forwarded-For` entry, then `X-Real-IP`) when keying the admin/login
    /// rate limiter.
    ///
    /// When `false` (default), the rate-limit bucket is keyed on the socket peer
    /// address and forwarded headers are ignored — the safe default, since any
    /// client can forge `X-Forwarded-For` to evade or poison rate buckets. When
    /// `true`, each real client behind a trusted reverse proxy is bucketed by the
    /// rightmost forwarded IP (the entry the closest proxy appended, which the
    /// client cannot forge) instead of all clients sharing the proxy's single IP.
    /// Enable (`TOPGUN_TRUSTED_PROXY=1`) only when a trusted proxy that
    /// appends/overwrites the forwarding header sits in front of the server.
    pub trust_forwarded_for: bool,
    /// External auth providers for token exchange at POST /api/auth/token.
    /// An empty list disables the endpoint (returns 404 on all requests).
    /// Constructed programmatically from environment variables or config in
    /// `module.rs`; the Serialize/Deserialize derives on `AuthProviderConfig`
    /// are included for future config-file support.
    pub auth_providers: Vec<AuthProviderConfig>,
    /// When `true`, detailed auth error messages are forwarded to clients.
    /// Defaults to `false` (production-safe: generic "Authentication failed" is returned).
    /// Set `INSECURE_FORWARD_AUTH_ERRORS=true` in the environment for development debugging.
    pub insecure_forward_auth_errors: bool,
    /// Expected JWT `iss` (issuer) for request-path token validation. When `Some`,
    /// the WS handshake and HTTP `/sync` extractor reject tokens whose `iss` does
    /// not match. When `None` (default), issuer is not checked — correct for
    /// TopGun-minted tokens. Set `TOPGUN_JWT_ISSUER` to enforce, which is required
    /// when `jwt_secret` is pointed at a shared `IdP` key (otherwise that `IdP`'s
    /// tokens for any audience are accepted — audit F7).
    pub jwt_issuer: Option<String>,
    /// Expected JWT `aud` (audience) for request-path token validation. When
    /// `Some`, the WS handshake and HTTP `/sync` extractor reject tokens whose
    /// `aud` does not match and require the claim to be present. When `None`
    /// (default), audience is not checked. Set `TOPGUN_JWT_AUDIENCE` to enforce.
    pub jwt_audience: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 0,
            tls: None,
            connection: ConnectionConfig::default(),
            cors_origins: vec![],
            cors_max_age: Duration::from_secs(86_400),
            cors_allow_credentials: true,
            max_body_size: 2 * 1024 * 1024, // 2 MB
            request_timeout: Duration::from_secs(30),
            jwt_clock_skew_secs: 60,
            rate_limit_per_ip: 100,
            rate_limit_burst: 50,
            trust_forwarded_for: false,
            auth_providers: vec![],
            insecure_forward_auth_errors: false,
            jwt_issuer: None,
            jwt_audience: None,
        }
    }
}

/// TLS certificate configuration.
///
/// No `Default` impl because certificate paths have no sensible defaults.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Path to the TLS certificate file.
    pub cert_path: PathBuf,
    /// Path to the TLS private key file.
    pub key_path: PathBuf,
    /// Optional path to a CA certificate for client verification.
    pub ca_cert_path: Option<PathBuf>,
}

/// Per-connection configuration controlling backpressure and timeouts.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Bounded mpsc channel capacity for outbound messages per connection.
    pub outbound_channel_capacity: usize,
    /// Duration after which an idle connection is considered stale.
    ///
    /// Enforced by the connection reaper: an authenticated connection whose
    /// last heartbeat (client `PING`) is older than this is closed. The
    /// `TopGun` client pings every 5 s, so the 60 s default tolerates many
    /// missed pings before reaping a half-open or dead peer.
    pub idle_timeout: Duration,
    /// Maximum time an unauthenticated connection may exist before it is
    /// closed.
    ///
    /// Bounds the Phase-1 auth window so a client that connects and never
    /// completes authentication (or dribbles malformed frames) cannot hold a
    /// connection slot indefinitely (slowloris). Enforced both by a per-frame
    /// deadline in the WebSocket auth loop and as a reaper backstop.
    pub auth_timeout: Duration,
    /// How often the connection reaper scans the registry for stale
    /// connections. Should be well below `idle_timeout` so reaping latency is
    /// bounded by roughly `idle_timeout + reaper_interval`.
    pub reaper_interval: Duration,
    /// WebSocket write buffer size in bytes.
    pub ws_write_buffer_size: usize,
    /// Maximum WebSocket write buffer size in bytes.
    pub ws_max_write_buffer_size: usize,
    /// Maximum size in bytes of a complete inbound WebSocket message. Frames
    /// exceeding this are rejected by the transport before the handler decodes
    /// them, bounding the unauthenticated memory-spike surface and capping the
    /// input to the depth-checked decoder. Defaults to 2 MB to match the HTTP
    /// `/sync` body limit.
    pub ws_max_message_size: usize,
    /// Maximum size in bytes of a single inbound WebSocket frame (a message may
    /// span multiple frames). Defaults to 2 MB.
    pub ws_max_frame_size: usize,
    /// Consecutive live-event broadcasts dropped (outbound channel full) before
    /// a slow client connection is disconnected to force reconnect + Merkle
    /// resync, instead of letting it diverge silently. Reset on any successful
    /// broadcast, so a connection that drains at all is never disconnected by
    /// this — only one stuck for `threshold` events in a row. 0 disables the
    /// disconnect (pure best-effort drop). Defaults to 256 (one full outbound
    /// buffer's worth of consecutive misses).
    pub slow_consumer_drop_threshold: u64,
    /// Sustained inbound op-rate ceiling, in ops/second, applied per WebSocket
    /// connection on the data plane (Phase-2 ops). Layered on top of aggregate
    /// load shedding so a single abusive peer cannot saturate the in-flight
    /// pipeline for everyone; over-rate ops get a 429 back-off without tearing
    /// the connection down. 0 disables per-connection limiting. The default
    /// (20 000) leaves large headroom over realistic client and benchmark
    /// per-connection rates while still bounding a flood.
    pub data_plane_max_ops_per_sec: u32,
    /// Burst capacity for the per-connection inbound op-rate limiter (the most
    /// ops acceptable instantaneously after an idle period). Defaults to 40 000
    /// (2× the sustained rate).
    pub data_plane_ops_burst: u32,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            outbound_channel_capacity: 256,
            idle_timeout: Duration::from_secs(60),
            auth_timeout: Duration::from_secs(10),
            reaper_interval: Duration::from_secs(10),
            ws_write_buffer_size: 131_072,        // 128 KB
            ws_max_write_buffer_size: 524_288,    // 512 KB
            ws_max_message_size: 2 * 1024 * 1024, // 2 MB — matches HTTP /sync body limit
            ws_max_frame_size: 2 * 1024 * 1024,   // 2 MB
            slow_consumer_drop_threshold: 256,    // == default outbound capacity
            data_plane_max_ops_per_sec: 20_000,   // generous; bounds a flooding peer
            data_plane_ops_burst: 40_000,         // 2× sustained
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_config_defaults() {
        let config = NetworkConfig::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 0);
        assert!(config.tls.is_none());
        // Default CORS allowlist is empty to reject all cross-origin requests
        // by default; operators must explicitly configure allowed origins.
        assert!(config.cors_origins.is_empty());
        assert_eq!(config.cors_max_age, Duration::from_secs(86_400));
        assert!(config.cors_allow_credentials);
        assert_eq!(config.max_body_size, 2 * 1024 * 1024);
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.jwt_clock_skew_secs, 60);
        assert_eq!(config.rate_limit_per_ip, 100);
        assert_eq!(config.rate_limit_burst, 50);
        // Default is false — forwarded headers are not trusted unless an operator
        // asserts a trusted proxy via TOPGUN_TRUSTED_PROXY (anti-spoof default).
        assert!(!config.trust_forwarded_for);
        // Default is false — production-safe by default.
        assert!(!config.insecure_forward_auth_errors);
    }

    #[test]
    fn connection_config_defaults() {
        let config = ConnectionConfig::default();
        assert_eq!(config.outbound_channel_capacity, 256);
        assert_eq!(config.idle_timeout, Duration::from_secs(60));
        assert_eq!(config.auth_timeout, Duration::from_secs(10));
        assert_eq!(config.reaper_interval, Duration::from_secs(10));
        assert_eq!(config.ws_write_buffer_size, 131_072);
        assert_eq!(config.ws_max_write_buffer_size, 524_288);
        assert_eq!(config.ws_max_message_size, 2 * 1024 * 1024);
        assert_eq!(config.ws_max_frame_size, 2 * 1024 * 1024);
        assert_eq!(config.slow_consumer_drop_threshold, 256);
        assert_eq!(config.data_plane_max_ops_per_sec, 20_000);
        assert_eq!(config.data_plane_ops_burst, 40_000);
    }

    #[test]
    fn tls_config_no_default() {
        // TlsConfig intentionally has no Default -- verify it can be constructed manually
        let tls = TlsConfig {
            cert_path: PathBuf::from("/tmp/cert.pem"),
            key_path: PathBuf::from("/tmp/key.pem"),
            ca_cert_path: None,
        };
        assert_eq!(tls.cert_path, PathBuf::from("/tmp/cert.pem"));
        assert!(tls.ca_cert_path.is_none());
    }
}
