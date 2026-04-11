//! Write validation layer: authentication, map-level ACL, value size limits, and HLC sanitization.
//!
//! The security layer intercepts all client write operations BEFORE they reach CRDT merge:
//!
//! ```text
//! Client write -> Auth check -> Map ACL check -> Size check -> HLC sanitize -> CRDT merge
//! ```
//!
//! Operations from trusted origins (`Forwarded`, `Backup`, `Wan`, `System`) bypass all checks.

use std::sync::Arc;

use parking_lot::Mutex;
use topgun_core::{HLC, Timestamp};

use crate::network::connection::{ConnectionMetadata, MapPermissions};
use crate::service::operation::{CallerOrigin, OperationContext, OperationError};

// ---------------------------------------------------------------------------
// SecurityConfig
// ---------------------------------------------------------------------------

/// Configuration for the server-side write validation layer.
///
/// Default is intentionally permissive (`require_auth: false`, unlimited size,
/// default read+write permissions) to preserve backward compatibility for
/// deployments without security configuration.
#[derive(Debug, Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    /// When true, all write operations require `ConnectionMetadata.authenticated == true`.
    /// When false, unauthenticated connections are allowed to write (development mode).
    pub require_auth: bool,
    /// Maximum serialized value size in bytes for a single `ClientOp` record.
    /// 0 means unlimited. Uses `u64` (not `usize`) so this value is stable across
    /// 32-bit and 64-bit platforms and can be safely stored in config files.
    pub max_value_bytes: u64,
    /// Default permissions for maps not explicitly configured per-connection.
    pub default_permissions: MapPermissions,
}

// ---------------------------------------------------------------------------
// WriteValidator
// ---------------------------------------------------------------------------

/// Validates client write operations before they reach CRDT merge.
///
/// Checks are applied in order:
/// 1. Caller origin bypass (trusted server-to-server traffic skips all checks)
/// 2. Authentication check (if `require_auth` is enabled)
/// 3. Map-level ACL check (read/write permissions from `ConnectionMetadata`)
/// 4. Value size check (against `max_value_bytes`)
///
/// After validation passes, `sanitize_hlc()` generates a fresh server-side
/// HLC timestamp that replaces the client-provided timestamp in the stored record.
pub struct WriteValidator {
    config: Arc<SecurityConfig>,
    hlc: Arc<Mutex<HLC>>,
}

impl WriteValidator {
    /// Creates a new `WriteValidator` with the given security configuration and server HLC.
    #[must_use]
    pub fn new(config: Arc<SecurityConfig>, hlc: Arc<Mutex<HLC>>) -> Self {
        Self { config, hlc }
    }

    /// Validates a write operation against the security policy.
    ///
    /// # Arguments
    ///
    /// - `ctx` — operation context, used for caller origin check
    /// - `metadata` — snapshot of the connection's metadata (auth state, map permissions)
    /// - `map_name` — target map for the write
    /// - `value_size` — serialized byte length of the record payload; pass `0` for `REMOVE`/`OR_REMOVE` ops
    ///
    /// # Errors
    ///
    /// - `OperationError::Unauthorized` — connection is not authenticated when `require_auth` is true
    /// - `OperationError::Forbidden` — connection lacks write permission for `map_name`
    /// - `OperationError::ValueTooLarge` — record exceeds `max_value_bytes`
    pub fn validate_write(
        &self,
        ctx: &OperationContext,
        metadata: &ConnectionMetadata,
        map_name: &str,
        value_size: u64,
    ) -> Result<(), OperationError> {
        // Anonymous callers are denied writes unconditionally as defense-in-depth.
        // Even if RBAC allows an anonymous read, writes are never permitted without
        // an authenticated identity.
        if matches!(ctx.caller_origin, CallerOrigin::Anonymous) {
            return Err(OperationError::Forbidden {
                map_name: map_name.to_string(),
            });
        }

        // Trusted server-to-server traffic bypasses all checks. HttpClient
        // falls through to the same ACL/size checks as Client; its auth
        // enforcement is handler-level (HTTP 401 before dispatch) rather
        // than metadata-based.
        if !matches!(ctx.caller_origin, CallerOrigin::Client | CallerOrigin::HttpClient) {
            return Ok(());
        }

        // Authentication check via connection metadata. HTTP operations have
        // no connection_id and therefore no ConnectionMetadata, so this check
        // is skipped for HttpClient (no metadata is passed through for them
        // because the CRDT service gates validate_write on connection_id presence).
        if self.config.require_auth && !metadata.authenticated {
            return Err(OperationError::Unauthorized);
        }

        // Map-level ACL check: look up per-connection permissions, fall back to default.
        let permissions = metadata
            .map_permissions
            .get(map_name)
            .copied()
            .unwrap_or(self.config.default_permissions);

        if !permissions.write {
            return Err(OperationError::Forbidden {
                map_name: map_name.to_string(),
            });
        }

        // Value size check (0 means unlimited).
        if self.config.max_value_bytes > 0 && value_size > self.config.max_value_bytes {
            return Err(OperationError::ValueTooLarge {
                size: value_size,
                max: self.config.max_value_bytes,
            });
        }

        Ok(())
    }

    /// Generates a fresh server-side HLC timestamp.
    ///
    /// This timestamp replaces the client-provided timestamp in stored records,
    /// preventing clients from manipulating HLC values to win future LWW conflicts.
    #[must_use]
    pub fn sanitize_hlc(&self) -> Timestamp {
        self.hlc.lock().now()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::default_trait_access)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Instant;

    use parking_lot::Mutex;
    use topgun_core::{HLC, SystemClock};

    use super::*;
    use crate::network::connection::MapPermissions;
    use crate::service::operation::{CallerOrigin, OperationContext, service_names};

    fn make_hlc() -> Arc<Mutex<HLC>> {
        Arc::new(Mutex::new(HLC::new("test-node".to_string(), Box::new(SystemClock))))
    }

    fn make_timestamp() -> topgun_core::Timestamp {
        topgun_core::Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "client-node".to_string(),
        }
    }

    fn make_ctx_client() -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Client;
        ctx
    }

    fn make_ctx_forwarded() -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Forwarded;
        ctx
    }

    fn make_ctx_system() -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::System;
        ctx
    }

    fn make_metadata(authenticated: bool) -> ConnectionMetadata {
        ConnectionMetadata {
            authenticated,
            principal: None,
            subscriptions: Default::default(),
            topics: Default::default(),
            last_heartbeat: Instant::now(),
            last_hlc: None,
            peer_node_id: None,
            map_permissions: HashMap::new(),
        }
    }

    fn make_validator(config: SecurityConfig) -> WriteValidator {
        WriteValidator::new(Arc::new(config), make_hlc())
    }

    // -- AC11: default config allows unauthenticated writes --

    #[test]
    fn default_config_allows_unauthenticated_writes() {
        let validator = make_validator(SecurityConfig::default());
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        assert!(validator.validate_write(&ctx, &metadata, "my-map", 0).is_ok());
    }

    // -- AC1: require_auth + unauthenticated => Unauthorized --

    #[test]
    fn require_auth_rejects_unauthenticated() {
        let config = SecurityConfig {
            require_auth: true,
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        let result = validator.validate_write(&ctx, &metadata, "my-map", 0);
        assert!(matches!(result, Err(OperationError::Unauthorized)));
    }

    // -- AC3: require_auth + authenticated + write perm => Ok --

    #[test]
    fn require_auth_allows_authenticated_with_write_perm() {
        let config = SecurityConfig {
            require_auth: true,
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(true);
        assert!(validator.validate_write(&ctx, &metadata, "my-map", 0).is_ok());
    }

    // -- AC2: authenticated + no write perm => Forbidden --

    #[test]
    fn no_write_permission_returns_forbidden() {
        let config = SecurityConfig {
            require_auth: true,
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let mut metadata = make_metadata(true);
        metadata.map_permissions.insert(
            "locked-map".to_string(),
            MapPermissions { read: true, write: false },
        );
        let result = validator.validate_write(&ctx, &metadata, "locked-map", 0);
        assert!(matches!(result, Err(OperationError::Forbidden { map_name }) if map_name == "locked-map"));
    }

    // -- AC6: value exceeds max_value_bytes => ValueTooLarge --

    #[test]
    fn oversized_value_returns_value_too_large() {
        let config = SecurityConfig {
            max_value_bytes: 100,
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        let result = validator.validate_write(&ctx, &metadata, "my-map", 101);
        assert!(matches!(result, Err(OperationError::ValueTooLarge { size: 101, max: 100 })));
    }

    // -- AC7: value exactly at max_value_bytes => Ok --

    #[test]
    fn exactly_max_value_bytes_succeeds() {
        let config = SecurityConfig {
            max_value_bytes: 100,
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        assert!(validator.validate_write(&ctx, &metadata, "my-map", 100).is_ok());
    }

    // -- AC12: max_value_bytes=0 means unlimited --

    #[test]
    fn zero_max_value_bytes_means_unlimited() {
        let config = SecurityConfig {
            max_value_bytes: 0,
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        assert!(validator.validate_write(&ctx, &metadata, "my-map", u64::MAX).is_ok());
    }

    // -- AC9: Forwarded origin bypasses all checks --

    #[test]
    fn forwarded_origin_bypasses_all_checks() {
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 1,
            default_permissions: MapPermissions { read: false, write: false },
        };
        let validator = make_validator(config);
        let ctx = make_ctx_forwarded();
        let metadata = make_metadata(false); // not authenticated
        // value_size=1000 > max_value_bytes=1, but bypass skips it
        assert!(validator.validate_write(&ctx, &metadata, "my-map", 1000).is_ok());
    }

    // -- AC10: System origin bypasses all checks --

    #[test]
    fn system_origin_bypasses_all_checks() {
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 1,
            default_permissions: MapPermissions { read: false, write: false },
        };
        let validator = make_validator(config);
        let ctx = make_ctx_system();
        let metadata = make_metadata(false);
        assert!(validator.validate_write(&ctx, &metadata, "my-map", 1000).is_ok());
    }

    // -- AC13: MapPermissions defaults to read=true, write=true --

    #[test]
    fn map_permissions_default_is_read_write() {
        let perms = MapPermissions::default();
        assert!(perms.read);
        assert!(perms.write);
    }

    // -- AC14: map_permissions defaults to empty HashMap --

    #[test]
    fn connection_metadata_map_permissions_defaults_to_empty() {
        let meta = ConnectionMetadata::default();
        assert!(meta.map_permissions.is_empty());
    }

    // -- AC15: sanitize_hlc returns timestamp with server's node_id --

    #[test]
    fn sanitize_hlc_returns_server_node_id() {
        let hlc = Arc::new(Mutex::new(HLC::new("server-node".to_string(), Box::new(SystemClock))));
        let validator = WriteValidator::new(Arc::new(SecurityConfig::default()), hlc);
        let ts = validator.sanitize_hlc();
        assert_eq!(ts.node_id, "server-node");
    }

    // -- AC16: successive sanitize_hlc calls produce monotonically increasing timestamps --

    #[test]
    fn successive_sanitize_hlc_calls_are_monotonic() {
        let validator = WriteValidator::new(Arc::new(SecurityConfig::default()), make_hlc());
        let ts1 = validator.sanitize_hlc();
        let ts2 = validator.sanitize_hlc();
        // Counter or millis must advance
        let t1 = (ts1.millis, ts1.counter);
        let t2 = (ts2.millis, ts2.counter);
        assert!(t2 >= t1, "timestamps must be monotonically non-decreasing");
        // Specifically, t2 must be strictly greater (HLC always advances)
        assert!(t2 > t1 || ts2.counter > ts1.counter, "HLC must strictly advance");
    }

    // -- Default permissions fallback --

    #[test]
    fn falls_back_to_default_permissions_when_map_not_in_per_connection_map() {
        let config = SecurityConfig {
            default_permissions: MapPermissions { read: true, write: false },
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(true); // no per-map permissions set
        let result = validator.validate_write(&ctx, &metadata, "any-map", 0);
        assert!(matches!(result, Err(OperationError::Forbidden { .. })));
    }

    // -- Anonymous origin is always rejected --

    #[test]
    fn anonymous_origin_is_rejected_with_forbidden() {
        let validator = make_validator(SecurityConfig::default());
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        let metadata = make_metadata(false);
        let result = validator.validate_write(&ctx, &metadata, "my-map", 0);
        assert!(matches!(result, Err(OperationError::Forbidden { map_name }) if map_name == "my-map"));
    }

    // -- Per-connection map permissions override default --

    #[test]
    fn per_connection_permissions_override_default() {
        let config = SecurityConfig {
            default_permissions: MapPermissions { read: true, write: false }, // default: no write
            ..SecurityConfig::default()
        };
        let validator = make_validator(config);
        let ctx = make_ctx_client();
        let mut metadata = make_metadata(true);
        // Explicitly grant write for "allowed-map"
        metadata.map_permissions.insert(
            "allowed-map".to_string(),
            MapPermissions { read: true, write: true },
        );
        // "allowed-map" should succeed despite default denying writes
        assert!(validator.validate_write(&ctx, &metadata, "allowed-map", 0).is_ok());
        // "other-map" falls back to default (no write)
        let result = validator.validate_write(&ctx, &metadata, "other-map", 0);
        assert!(matches!(result, Err(OperationError::Forbidden { .. })));
    }
}
