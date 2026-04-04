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
    /// External auth providers for token exchange at POST /api/auth/token.
    /// An empty list disables the endpoint (returns 404 on all requests).
    /// Constructed programmatically from environment variables or config in
    /// `module.rs`; the Serialize/Deserialize derives on `AuthProviderConfig`
    /// are included for future config-file support.
    pub auth_providers: Vec<AuthProviderConfig>,
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
            auth_providers: vec![],
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
    /// Maximum time to wait when sending a message to a connection.
    pub send_timeout: Duration,
    /// Duration after which an idle connection is considered stale.
    pub idle_timeout: Duration,
    /// WebSocket write buffer size in bytes.
    pub ws_write_buffer_size: usize,
    /// Maximum WebSocket write buffer size in bytes.
    pub ws_max_write_buffer_size: usize,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            outbound_channel_capacity: 256,
            send_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(60),
            ws_write_buffer_size: 131_072,     // 128 KB
            ws_max_write_buffer_size: 524_288,  // 512 KB
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
    }

    #[test]
    fn connection_config_defaults() {
        let config = ConnectionConfig::default();
        assert_eq!(config.outbound_channel_capacity, 256);
        assert_eq!(config.send_timeout, Duration::from_secs(5));
        assert_eq!(config.idle_timeout, Duration::from_secs(60));
        assert_eq!(config.ws_write_buffer_size, 131_072);
        assert_eq!(config.ws_max_write_buffer_size, 524_288);
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
