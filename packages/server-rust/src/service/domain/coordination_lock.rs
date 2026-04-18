//! Distributed lock registry for the coordination domain.
//!
//! Provides `LockRegistry`, an in-memory, DashMap-backed store of named
//! exclusive leases with optional TTL-driven expiry and connection-scoped
//! cleanup.
//!
//! # Contract Invariants
//!
//! **I1 — Response totality:** every `LOCK_REQUEST` produces exactly one wire
//! response, either `LOCK_GRANTED` (acquired, including idempotent re-acquire)
//! or `LOCK_RELEASED { success: false }` (denied: busy, invalid name, or
//! `ttl: 0` rejection). No silent path.
//!
//! **I2 — Fence monotonicity across holders:** for a given lock name, across
//! any sequence of acquire → release → acquire events (including TTL-driven
//! and disconnect-driven releases), successive fencing tokens are strictly
//! increasing. Holders that observe a fencing token `N` can rely on "a
//! subsequent grant will have token > N" indefinitely within a process
//! lifetime.
//!
//! **I3 — Same fence on re-entrant acquire:** a `try_acquire` by the CURRENT
//! holder (matching `ConnectionId`) returns the SAME fencing token previously
//! issued for that acquisition. A new token is NOT minted. Downstream users
//! that gate writes on fencing tokens remain stable under client retry.
//!
//! **I4 — Lazy TTL check:** `try_acquire` first evaluates
//! `entry.expires_at < Instant::now()` on a non-self-held entry; if true,
//! the entry is treated as free and reclaimed immediately. The tokio timer
//! remains for proactive cleanup but is not the sole correctness mechanism.
//!
//! **I5 — TTL=0 rejection (chosen semantics):** `LOCK_REQUEST { ttl: 0 }` is
//! explicitly rejected with `LOCK_RELEASED { success: false }`. `ttl: 0`
//! ("no expiry") is a leak footgun; clients who want long-lived locks MUST
//! send `ttl: 3_600_000` (1h) or similar and re-acquire.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Instant;

use dashmap::{DashMap, DashSet};
use tokio::task::JoinHandle;

use crate::network::connection::ConnectionId;

// ---------------------------------------------------------------------------
// Public error and outcome types
// ---------------------------------------------------------------------------

/// Errors specific to lock operations.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Lock name fails validation (empty, too long, or invalid characters).
    #[error("invalid lock name: {name:?}")]
    InvalidLockName { name: String },

    /// TTL value is explicitly rejected. Currently only `ttl: 0` triggers
    /// this; future stricter bounds (e.g., `ttl > 24h`) may reuse this
    /// variant. Carries the rejected value for observability.
    #[error("invalid ttl: {ttl_ms}")]
    InvalidTtl { ttl_ms: u64 },
}

/// Outcome of a `try_acquire` call. The handler translates each variant
/// into exactly one wire response (invariant I1 — response totality).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockOutcome {
    /// Lock granted; includes the fencing token issued to the caller.
    /// Emitted as `LOCK_GRANTED { fencing_token }`.
    Granted { fencing_token: u64 },
    /// Lock is currently held by another holder.
    /// Emitted as `LOCK_RELEASED { success: false }`.
    Busy,
}

// ---------------------------------------------------------------------------
// Internal per-lock state
// ---------------------------------------------------------------------------

/// Per-lock runtime state. Internal to the registry.
struct LockEntry {
    /// Connection holding the lock.
    holder: ConnectionId,
    /// Fencing token issued for this acquisition.
    fencing_token: u64,
    /// Monotonic instant when the lease expires (None = no TTL).
    expires_at: Option<Instant>,
    /// TTL-driven expiry task handle (None = no TTL).
    expiry_task: Option<JoinHandle<()>>,
}

// ---------------------------------------------------------------------------
// LockRegistry
// ---------------------------------------------------------------------------

/// Thread-safe in-memory distributed lock registry.
///
/// Keyed by lock name. Each lock tracks its holder, fencing token, and
/// optional TTL-driven expiry. A reverse-index (connection -> held lock
/// names) supports efficient disconnect cleanup.
pub struct LockRegistry {
    /// Lock entries keyed by lock name.
    locks: DashMap<String, LockEntry>,
    /// Reverse-index: connection -> set of lock names held.
    held_by: DashMap<ConnectionId, DashSet<String>>,
    /// Per-lock-name monotonic fencing counter. Separate map so counters
    /// survive lock release (next acquire gets a strictly higher token).
    fencing_counters: DashMap<String, Arc<AtomicU64>>,
}

impl LockRegistry {
    /// Creates a new empty lock registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            locks: DashMap::new(),
            held_by: DashMap::new(),
            fencing_counters: DashMap::new(),
        }
    }

    /// Validates a lock name against the same pattern as topic names:
    /// non-empty, max 256 characters, only `[A-Za-z0-9_\-.:/]`.
    fn validate_lock_name(name: &str) -> Result<(), LockError> {
        todo!("validate_lock_name: check non-empty, max 256 chars, allowed charset")
    }

    /// Attempts to acquire a lock. Returns `Granted { fencing_token }` on
    /// success, or `Busy` if held by another holder. Re-acquire by the
    /// current holder returns the existing fencing token (idempotent) — see
    /// invariant I3.
    ///
    /// **Lazy TTL check (invariant I4):** before returning `Busy` for an
    /// entry held by a different holder, evaluates `entry.expires_at <
    /// Instant::now()`; if true, evicts the stale entry and proceeds with
    /// acquisition. Pattern ported from TiKV's
    /// `check_txn_status.rs:55` — belt-and-suspenders against tokio timer
    /// scheduling delay under load. The tokio expiry task still exists for
    /// proactive cleanup.
    ///
    /// # Errors
    /// - `LockError::InvalidLockName` if the name fails validation.
    /// - `LockError::InvalidTtl { ttl_ms: 0 }` if `ttl_ms == Some(0)` —
    ///   chosen semantics per invariant I5. `None` (no TTL at all) is
    ///   accepted but emits a `tracing::warn!` because never-expiring
    ///   locks on disconnect-not-yet-fired holders are a leak risk.
    ///
    /// On success with a `Some(ttl_ms)` where `ttl_ms > 0`, spawns a tokio
    /// task that calls `expire_if_token_matches(name, token)` after
    /// `ttl_ms` milliseconds.
    ///
    /// `ttl_ms` mirrors wire payload.ttl (already in milliseconds).
    pub fn try_acquire(
        self: &Arc<Self>,
        name: &str,
        holder: ConnectionId,
        ttl_ms: Option<u64>,
    ) -> Result<LockOutcome, LockError> {
        todo!("try_acquire: validate, check lazy TTL, acquire or return Busy/Granted")
    }

    /// Releases a lock if the fencing token matches the currently-held
    /// token. Returns `true` on successful release, `false` if the name
    /// is not held or the token does not match.
    ///
    /// Cancels any pending TTL expiry task.
    pub fn release(&self, name: &str, fencing_token: u64) -> bool {
        todo!("release: match token, cancel expiry task, clean reverse-index")
    }

    /// TTL-driven expiry path. Called by the spawned tokio timer task.
    /// Releases the lock IFF it is still held and its fencing token
    /// matches `expected_token` (protects against race where explicit
    /// release + re-acquire happened before the timer fired).
    fn expire_if_token_matches(&self, name: &str, expected_token: u64) {
        todo!("expire_if_token_matches: token-guarded eviction from locks + held_by")
    }

    /// Releases all locks held by the given connection. Called on
    /// connection disconnect. Cancels TTL expiry tasks for released
    /// entries.
    pub fn release_on_disconnect(&self, conn_id: ConnectionId) {
        todo!("release_on_disconnect: iterate held_by[conn_id], release each lock")
    }

    /// Returns the current holder of a lock, if any (for testing).
    pub fn holder(&self, name: &str) -> Option<ConnectionId> {
        todo!("holder: look up locks[name].holder")
    }

    /// Returns the current fencing token for a lock, if held (for testing).
    pub fn current_token(&self, name: &str) -> Option<u64> {
        todo!("current_token: look up locks[name].fencing_token")
    }

    /// Returns the number of active locks (for testing).
    pub fn lock_count(&self) -> usize {
        todo!("lock_count: locks.len()")
    }
}

impl Default for LockRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Tests will be added in G2a and G2b waves.
}
