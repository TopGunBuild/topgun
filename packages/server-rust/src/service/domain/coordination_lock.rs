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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::{DashMap, DashSet};
use tokio::task::JoinHandle;
use tokio::time::Instant;

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
        if name.is_empty() || name.len() > 256 {
            return Err(LockError::InvalidLockName {
                name: name.to_string(),
            });
        }
        let valid = name.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' || c == '/'
        });
        if !valid {
            return Err(LockError::InvalidLockName {
                name: name.to_string(),
            });
        }
        Ok(())
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
        Self::validate_lock_name(name)?;

        // Invariant I5: reject ttl: 0 explicitly.
        if ttl_ms == Some(0) {
            return Err(LockError::InvalidTtl { ttl_ms: 0 });
        }

        if ttl_ms.is_none() {
            tracing::warn!(
                lock_name = name,
                "lock acquired with no TTL — disconnect cleanup is the only release path"
            );
        }

        // Invariant I4 + I3: check existing entry.
        if let Some(mut entry) = self.locks.get_mut(name) {
            if entry.holder == holder {
                // Invariant I3: same holder re-acquires — return existing token.
                return Ok(LockOutcome::Granted {
                    fencing_token: entry.fencing_token,
                });
            }

            // Different holder: check lazy TTL (invariant I4).
            let is_expired = entry
                .expires_at
                .map(|exp| exp <= Instant::now())
                .unwrap_or(false);

            if !is_expired {
                // Lock is actively held by another holder.
                return Ok(LockOutcome::Busy);
            }

            // Lazy eviction: cancel the stale entry's expiry task and fall through
            // to acquire below. We need to drop the lock entry guard first.
            if let Some(task) = entry.expiry_task.take() {
                task.abort();
            }
            let stale_holder = entry.holder;
            drop(entry);

            // Clean up reverse-index for the stale holder.
            self.locks.remove(name);
            if let Some(held) = self.held_by.get(&stale_holder) {
                held.remove(name);
            }
        }

        // Mint a new fencing token (strictly monotonic, invariant I2).
        let counter = self
            .fencing_counters
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        let fencing_token = counter.fetch_add(1, Ordering::Relaxed) + 1;

        // Compute expiry instant and spawn TTL task if requested.
        let expires_at = ttl_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        let expiry_task = if let Some(ms) = ttl_ms {
            let registry = Arc::clone(self);
            let name_owned = name.to_string();
            Some(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                registry.expire_if_token_matches(&name_owned, fencing_token);
            }))
        } else {
            None
        };

        // Insert new entry.
        self.locks.insert(
            name.to_string(),
            LockEntry {
                holder,
                fencing_token,
                expires_at,
                expiry_task,
            },
        );

        // Update reverse-index.
        self.held_by
            .entry(holder)
            .or_insert_with(DashSet::new)
            .insert(name.to_string());

        Ok(LockOutcome::Granted { fencing_token })
    }

    /// Releases a lock if the fencing token matches the currently-held
    /// token. Returns `true` on successful release, `false` if the name
    /// is not held or the token does not match.
    ///
    /// Cancels any pending TTL expiry task.
    pub fn release(&self, name: &str, fencing_token: u64) -> bool {
        // Use remove_if to atomically check and remove the entry.
        let removed = self
            .locks
            .remove_if(name, |_, entry| entry.fencing_token == fencing_token);

        if let Some((_, mut entry)) = removed {
            // Cancel TTL task to prevent late-fire race.
            if let Some(task) = entry.expiry_task.take() {
                task.abort();
            }
            // Clean up reverse-index.
            if let Some(held) = self.held_by.get(&entry.holder) {
                held.remove(name);
            }
            true
        } else {
            false
        }
    }

    /// TTL-driven expiry path. Called by the spawned tokio timer task.
    /// Releases the lock IFF it is still held and its fencing token
    /// matches `expected_token` (protects against race where explicit
    /// release + re-acquire happened before the timer fired).
    fn expire_if_token_matches(&self, name: &str, expected_token: u64) {
        let removed = self
            .locks
            .remove_if(name, |_, entry| entry.fencing_token == expected_token);

        if let Some((_, entry)) = removed {
            // Clean up reverse-index.
            if let Some(held) = self.held_by.get(&entry.holder) {
                held.remove(name);
            }
            // No need to abort expiry_task — we ARE the expiry task.
        }
    }

    /// Releases all locks held by the given connection. Called on
    /// connection disconnect. Cancels TTL expiry tasks for released
    /// entries.
    pub fn release_on_disconnect(&self, conn_id: ConnectionId) {
        let Some((_, held_names)) = self.held_by.remove(&conn_id) else {
            return;
        };

        // Collect names to avoid holding DashSet reference during mutation.
        let names: Vec<String> = held_names.iter().map(|n| n.clone()).collect();

        for name in names {
            let removed = self
                .locks
                .remove_if(&name, |_, entry| entry.holder == conn_id);

            if let Some((_, mut entry)) = removed {
                if let Some(task) = entry.expiry_task.take() {
                    task.abort();
                }
            }
        }
    }

    /// Returns the current holder of a lock, if any (for testing).
    pub fn holder(&self, name: &str) -> Option<ConnectionId> {
        self.locks.get(name).map(|e| e.holder)
    }

    /// Returns the current fencing token for a lock, if held (for testing).
    pub fn current_token(&self, name: &str) -> Option<u64> {
        self.locks.get(name).map(|e| e.fencing_token)
    }

    /// Returns the number of active locks (for testing).
    pub fn lock_count(&self) -> usize {
        self.locks.len()
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
    use super::*;

    fn conn(id: u64) -> ConnectionId {
        ConnectionId(id)
    }

    fn registry() -> Arc<LockRegistry> {
        Arc::new(LockRegistry::new())
    }

    // -- Core acquisition tests --

    #[tokio::test]
    async fn try_acquire_returns_granted_for_free_lock() {
        let reg = registry();
        let outcome = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        assert!(matches!(outcome, LockOutcome::Granted { fencing_token } if fencing_token > 0));
        assert_eq!(reg.lock_count(), 1);
    }

    #[tokio::test]
    async fn try_acquire_returns_busy_for_held_lock() {
        let reg = registry();
        reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let outcome = reg.try_acquire("lock-a", conn(2), Some(5000)).unwrap();
        assert_eq!(outcome, LockOutcome::Busy);
    }

    #[tokio::test]
    async fn try_acquire_by_same_holder_is_idempotent() {
        let reg = registry();
        let first = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let second = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        // Invariant I3: same token returned.
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn try_acquire_ttl_zero_returns_invalid_ttl_error() {
        let reg = registry();
        let err = reg.try_acquire("lock-a", conn(1), Some(0)).unwrap_err();
        assert!(matches!(err, LockError::InvalidTtl { ttl_ms: 0 }));
    }

    // -- Release tests --

    #[tokio::test]
    async fn release_returns_true_with_matching_token() {
        let reg = registry();
        let outcome = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token } = outcome else {
            panic!("expected Granted");
        };
        assert!(reg.release("lock-a", fencing_token));
        assert_eq!(reg.lock_count(), 0);
    }

    #[tokio::test]
    async fn release_returns_false_with_stale_token() {
        let reg = registry();
        reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        // Token 999 was never issued.
        assert!(!reg.release("lock-a", 999));
        // Lock is still held.
        assert_eq!(reg.lock_count(), 1);
    }

    #[tokio::test]
    async fn release_returns_false_for_unknown_name() {
        let reg = registry();
        assert!(!reg.release("nonexistent", 1));
    }

    // -- TTL tests --

    #[tokio::test]
    async fn ttl_expires_lock_after_deadline() {
        // Acquire a lock with a short TTL, then verify the lock is automatically
        // released after the TTL by driving a real short wall-clock wait.
        // The TTL mechanism is tested end-to-end: the spawned expiry task fires
        // via tokio::time::sleep and calls expire_if_token_matches.
        let reg = registry();
        reg.try_acquire("lock-a", conn(1), Some(50)).unwrap();
        assert_eq!(reg.lock_count(), 1);

        // Wait for slightly longer than the TTL. Using tokio::time::sleep
        // here to let the spawned expiry task complete naturally.
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(reg.lock_count(), 0, "lock must be expired after TTL");
    }

    #[tokio::test]
    async fn explicit_release_cancels_pending_ttl_task() {
        let reg = registry();
        let outcome = reg.try_acquire("lock-a", conn(1), Some(50)).unwrap();
        let LockOutcome::Granted { fencing_token } = outcome else {
            panic!("expected Granted");
        };

        // Release before TTL fires.
        assert!(reg.release("lock-a", fencing_token));

        // New holder acquires with a longer TTL.
        let outcome2 = reg.try_acquire("lock-a", conn(2), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: token2 } = outcome2 else {
            panic!("expected Granted for new holder");
        };

        // Wait past the original 50ms TTL — the old timer was aborted on
        // explicit release, so the new holder's lock must still be held.
        tokio::time::sleep(Duration::from_millis(150)).await;

        // New holder's lock must still be held.
        assert_eq!(reg.holder("lock-a"), Some(conn(2)));
        assert_eq!(reg.current_token("lock-a"), Some(token2));
    }

    #[tokio::test(start_paused = true)]
    async fn try_acquire_lazy_ttl_check_reclaims_expired_lock_without_timer_firing() {
        let reg = registry();

        // First holder acquires with 100ms TTL.
        reg.try_acquire("lock-a", conn(1), Some(100)).unwrap();

        // Advance time past TTL WITHOUT yielding (timer task has not run).
        tokio::time::advance(Duration::from_millis(200)).await;
        // Do NOT yield here — the timer task should not have fired yet.

        // Second holder tries to acquire; lazy check reclaims the expired entry.
        // Invariant I4: belt-and-suspenders check.
        let outcome = reg.try_acquire("lock-a", conn(2), Some(5000)).unwrap();
        assert!(
            matches!(outcome, LockOutcome::Granted { .. }),
            "lazy TTL reclaim must grant lock to second holder"
        );
        assert_eq!(reg.holder("lock-a"), Some(conn(2)));
    }

    // -- Disconnect cleanup tests --

    #[tokio::test]
    async fn release_on_disconnect_releases_all_locks_for_connection() {
        let reg = registry();
        reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        reg.try_acquire("lock-b", conn(1), Some(5000)).unwrap();
        reg.try_acquire("lock-c", conn(2), Some(5000)).unwrap();

        reg.release_on_disconnect(conn(1));

        assert_eq!(reg.lock_count(), 1, "only conn(2)'s lock should remain");
        assert!(reg.holder("lock-a").is_none());
        assert!(reg.holder("lock-b").is_none());
        assert_eq!(reg.holder("lock-c"), Some(conn(2)));
    }

    #[tokio::test]
    async fn release_on_disconnect_does_not_affect_other_connections() {
        let reg = registry();
        reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        reg.try_acquire("lock-b", conn(2), Some(5000)).unwrap();

        reg.release_on_disconnect(conn(1));

        assert_eq!(reg.lock_count(), 1);
        assert_eq!(reg.holder("lock-b"), Some(conn(2)));
    }

    // -- Name validation tests --

    #[test]
    fn validate_lock_name_rejects_empty_too_long_and_invalid_chars() {
        // Empty.
        assert!(LockRegistry::validate_lock_name("").is_err());
        // Too long (257 chars).
        assert!(LockRegistry::validate_lock_name(&"a".repeat(257)).is_err());
        // Invalid character (space).
        assert!(LockRegistry::validate_lock_name("bad name").is_err());
        // Invalid character (!).
        assert!(LockRegistry::validate_lock_name("lock!").is_err());

        // Valid names.
        assert!(LockRegistry::validate_lock_name("lock-a").is_ok());
        assert!(LockRegistry::validate_lock_name("lock.a:b/c").is_ok());
        assert!(LockRegistry::validate_lock_name(&"a".repeat(256)).is_ok());
    }

    // -- Fencing counter persistence tests --

    #[tokio::test]
    async fn fencing_counter_persists_across_release() {
        let reg = registry();
        let outcome1 = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t1 } = outcome1 else {
            panic!("expected Granted");
        };
        assert!(reg.release("lock-a", t1));

        let outcome2 = reg.try_acquire("lock-a", conn(2), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t2 } = outcome2 else {
            panic!("expected Granted");
        };
        // Invariant I2: strictly monotonic.
        assert!(t2 > t1, "token must be strictly increasing after release");
    }

    #[tokio::test]
    async fn fencing_counter_persists_across_ttl_expiry() {
        let reg = registry();
        let outcome1 = reg.try_acquire("lock-a", conn(1), Some(50)).unwrap();
        let LockOutcome::Granted { fencing_token: t1 } = outcome1 else {
            panic!("expected Granted");
        };

        // Let the TTL expiry task fire via a real wait longer than the TTL.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(reg.lock_count(), 0);

        let outcome2 = reg.try_acquire("lock-a", conn(2), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t2 } = outcome2 else {
            panic!("expected Granted");
        };
        // Invariant I2: fence monotonic across TTL-driven release.
        assert!(t2 > t1);
    }

    #[tokio::test]
    async fn fencing_counter_persists_across_disconnect_release() {
        let reg = registry();
        let outcome1 = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t1 } = outcome1 else {
            panic!("expected Granted");
        };

        reg.release_on_disconnect(conn(1));

        let outcome2 = reg.try_acquire("lock-a", conn(2), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t2 } = outcome2 else {
            panic!("expected Granted");
        };
        // Invariant I2: fence monotonic across disconnect-driven release.
        assert!(t2 > t1);
    }

    #[tokio::test]
    async fn reacquire_returns_same_fence_invariant_i3() {
        let reg = registry();
        let outcome1 = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t1 } = outcome1 else {
            panic!("expected Granted");
        };
        // Same holder re-acquires: must return SAME token (I3).
        let outcome2 = reg.try_acquire("lock-a", conn(1), Some(5000)).unwrap();
        let LockOutcome::Granted { fencing_token: t2 } = outcome2 else {
            panic!("expected Granted");
        };
        assert_eq!(t1, t2, "re-entrant acquire must return same fencing token");
    }
}
