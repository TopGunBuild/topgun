//! Write admission layer: authentication baseline, value size limits, and HLC sanitization.
//!
//! This is an admission/integrity control at the edge, NOT an authorization layer.
//! It performs cheap, identity-agnostic gating before client writes reach CRDT merge:
//! deny anonymous writes, route trusted server-to-server traffic past the gate, enforce
//! an authentication baseline, bound payload size, and sanitize client-provided HLC
//! timestamps. Per-resource authorization (who may write which map/record) is owned
//! exclusively by the RBAC policy engine, which runs separately — admission makes no
//! identity-aware ACL decision.
//!
//! ```text
//! Client write -> Auth baseline -> Size check -> HLC sanitize -> CRDT merge
//! ```
//!
//! Operations from trusted origins (`Forwarded`, `Backup`, `Wan`, `System`) bypass all checks.

use std::sync::Arc;

use parking_lot::Mutex;
use topgun_core::{Timestamp, HLC};

use crate::network::connection::ConnectionMetadata;
use crate::service::operation::{CallerOrigin, OperationContext, OperationError};

// ---------------------------------------------------------------------------
// SecurityConfig
// ---------------------------------------------------------------------------

/// Configuration for the server-side write admission layer.
///
/// Default is intentionally permissive (`require_auth: false`, unlimited size)
/// to preserve backward compatibility for deployments without security configuration.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    /// When true, all write operations require `ConnectionMetadata.authenticated == true`.
    /// When false, unauthenticated connections are allowed to write (development mode).
    pub require_auth: bool,
    /// Maximum serialized value size in bytes for a single `ClientOp` record.
    /// 0 means unlimited. Uses `u64` (not `usize`) so this value is stable across
    /// 32-bit and 64-bit platforms and can be safely stored in config files.
    pub max_value_bytes: u64,
}

// ---------------------------------------------------------------------------
// WriteAdmission
// ---------------------------------------------------------------------------

/// Admits client write operations before they reach CRDT merge.
///
/// This is an identity-agnostic integrity gate, NOT an authorization layer.
/// Per-resource authorization is owned by the RBAC policy engine elsewhere.
///
/// Checks are applied in order:
/// 1. Anonymous-write deny (defense-in-depth: writes require an identity)
/// 2. Caller origin bypass (trusted server-to-server traffic skips all checks)
/// 3. Authentication baseline (if `require_auth` is enabled)
/// 4. Value size check (against `max_value_bytes`)
///
/// After admission passes, `sanitize_hlc()` generates a fresh server-side
/// HLC timestamp that replaces the client-provided timestamp in the stored record.
pub struct WriteAdmission {
    config: Arc<SecurityConfig>,
    hlc: Arc<Mutex<HLC>>,
}

impl WriteAdmission {
    /// Creates a new `WriteAdmission` with the given security configuration and server HLC.
    #[must_use]
    pub fn new(config: Arc<SecurityConfig>, hlc: Arc<Mutex<HLC>>) -> Self {
        Self { config, hlc }
    }

    /// Admits a write operation against the admission/integrity policy.
    ///
    /// # Arguments
    ///
    /// - `ctx` — operation context, used for caller origin check
    /// - `metadata` — snapshot of the connection's metadata (auth state)
    /// - `map_name` — target map for the write; used only for error context
    ///   (`Forbidden`/`ValueTooLarge`), not for any ACL decision
    /// - `value_size` — serialized byte length of the record payload; pass `0` for `REMOVE`/`OR_REMOVE` ops
    ///
    /// # Errors
    ///
    /// - `OperationError::Forbidden` — anonymous caller (writes require an identity)
    /// - `OperationError::Unauthorized` — connection is not authenticated when `require_auth` is true
    /// - `OperationError::ValueTooLarge` — record exceeds `max_value_bytes`
    pub fn admit_write(
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
        // falls through to the same auth/size checks as Client; its auth
        // enforcement is handler-level (HTTP 401 before dispatch) rather
        // than metadata-based.
        if !matches!(
            ctx.caller_origin,
            CallerOrigin::Client | CallerOrigin::HttpClient
        ) {
            return Ok(());
        }

        // Authentication baseline via connection metadata. WebSocket/Client writes
        // pass a snapshot of the live connection metadata; HttpClient writes have
        // no connection_id, so the CRDT service synthesizes a metadata snapshot
        // whose `authenticated` flag is derived from the JWT-validated principal
        // already attached to the operation context. The check therefore runs for
        // HttpClient too: it passes for legitimate (token-bearing) HTTP writes and
        // fail-closes if a principal is ever absent.
        if self.config.require_auth && !metadata.authenticated {
            return Err(OperationError::Unauthorized);
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
    use std::sync::Arc;
    use std::time::Instant;

    use parking_lot::Mutex;
    use topgun_core::{SystemClock, HLC};

    use super::*;
    use crate::service::operation::{service_names, CallerOrigin, OperationContext};

    fn make_hlc() -> Arc<Mutex<HLC>> {
        Arc::new(Mutex::new(HLC::new(
            "test-node".to_string(),
            Box::new(SystemClock),
        )))
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
            handshake_complete: authenticated,
            principal: None,
            subscriptions: Default::default(),
            topics: Default::default(),
            last_heartbeat: Instant::now(),
            last_hlc: None,
            peer_node_id: None,
            device_id: None,
        }
    }

    fn make_admission(config: SecurityConfig) -> WriteAdmission {
        WriteAdmission::new(Arc::new(config), make_hlc())
    }

    // -- Default config admits unauthenticated writes (permissive baseline) --

    #[test]
    fn default_config_admits_unauthenticated_writes() {
        let admission = make_admission(SecurityConfig::default());
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        assert!(admission.admit_write(&ctx, &metadata, "my-map", 0).is_ok());
    }

    // -- R8(a): require_auth + unauthenticated => Unauthorized --

    #[test]
    fn require_auth_rejects_unauthenticated() {
        let config = SecurityConfig {
            require_auth: true,
            ..SecurityConfig::default()
        };
        let admission = make_admission(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        let result = admission.admit_write(&ctx, &metadata, "my-map", 0);
        assert!(matches!(result, Err(OperationError::Unauthorized)));
    }

    // -- R8(b): require_auth + authenticated => Ok --

    #[test]
    fn require_auth_admits_authenticated() {
        let config = SecurityConfig {
            require_auth: true,
            ..SecurityConfig::default()
        };
        let admission = make_admission(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(true);
        assert!(admission.admit_write(&ctx, &metadata, "my-map", 0).is_ok());
    }

    // -- R8(c) + negative control: anonymous origin => Forbidden --

    #[test]
    fn anonymous_origin_is_rejected_with_forbidden() {
        // NEGATIVE CONTROL: removing the deny-anonymous guard in `admit_write`
        // makes this assertion fail (an anonymous write would be admitted Ok).
        // That green->red flip is the entire point of this test — it proves the
        // guard is load-bearing, not dead code.
        let admission = make_admission(SecurityConfig::default());
        let mut ctx = OperationContext::new(1, service_names::CRDT, make_timestamp(), 5000);
        ctx.caller_origin = CallerOrigin::Anonymous;
        let metadata = make_metadata(false);
        let result = admission.admit_write(&ctx, &metadata, "my-map", 0);
        assert!(
            matches!(result, Err(OperationError::Forbidden { map_name }) if map_name == "my-map")
        );
    }

    // -- R8(d): value exceeds max_value_bytes => ValueTooLarge --

    #[test]
    fn oversized_value_returns_value_too_large() {
        let config = SecurityConfig {
            max_value_bytes: 100,
            ..SecurityConfig::default()
        };
        let admission = make_admission(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        let result = admission.admit_write(&ctx, &metadata, "my-map", 101);
        assert!(matches!(
            result,
            Err(OperationError::ValueTooLarge {
                size: 101,
                max: 100
            })
        ));
    }

    // -- R8(e): value exactly at max_value_bytes => Ok --

    #[test]
    fn exactly_max_value_bytes_succeeds() {
        let config = SecurityConfig {
            max_value_bytes: 100,
            ..SecurityConfig::default()
        };
        let admission = make_admission(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        assert!(admission
            .admit_write(&ctx, &metadata, "my-map", 100)
            .is_ok());
    }

    // -- R8(f): max_value_bytes=0 means unlimited --

    #[test]
    fn zero_max_value_bytes_means_unlimited() {
        let config = SecurityConfig {
            max_value_bytes: 0,
            ..SecurityConfig::default()
        };
        let admission = make_admission(config);
        let ctx = make_ctx_client();
        let metadata = make_metadata(false);
        assert!(admission
            .admit_write(&ctx, &metadata, "my-map", u64::MAX)
            .is_ok());
    }

    // -- R8(g): Forwarded (trusted) origin bypasses all checks --

    #[test]
    fn forwarded_origin_bypasses_all_checks() {
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 1,
        };
        let admission = make_admission(config);
        let ctx = make_ctx_forwarded();
        let metadata = make_metadata(false); // not authenticated
                                             // value_size=1000 > max_value_bytes=1, but bypass skips it
        assert!(admission
            .admit_write(&ctx, &metadata, "my-map", 1000)
            .is_ok());
    }

    // -- R8(g): System (trusted) origin bypasses all checks --

    #[test]
    fn system_origin_bypasses_all_checks() {
        let config = SecurityConfig {
            require_auth: true,
            max_value_bytes: 1,
        };
        let admission = make_admission(config);
        let ctx = make_ctx_system();
        let metadata = make_metadata(false);
        assert!(admission
            .admit_write(&ctx, &metadata, "my-map", 1000)
            .is_ok());
    }

    // -- R8(h): sanitize_hlc returns timestamp with server's node_id --

    #[test]
    fn sanitize_hlc_returns_server_node_id() {
        let hlc = Arc::new(Mutex::new(HLC::new(
            "server-node".to_string(),
            Box::new(SystemClock),
        )));
        let admission = WriteAdmission::new(Arc::new(SecurityConfig::default()), hlc);
        let ts = admission.sanitize_hlc();
        assert_eq!(ts.node_id, "server-node");
    }

    // -- R8(h): successive sanitize_hlc calls produce monotonically increasing timestamps --

    #[test]
    fn successive_sanitize_hlc_calls_are_monotonic() {
        let admission = WriteAdmission::new(Arc::new(SecurityConfig::default()), make_hlc());
        let ts1 = admission.sanitize_hlc();
        let ts2 = admission.sanitize_hlc();
        // Counter or millis must advance
        let t1 = (ts1.millis, ts1.counter);
        let t2 = (ts2.millis, ts2.counter);
        assert!(t2 >= t1, "timestamps must be monotonically non-decreasing");
        // Specifically, t2 must be strictly greater (HLC always advances)
        assert!(
            t2 > t1 || ts2.counter > ts1.counter,
            "HLC must strictly advance"
        );
    }
}
