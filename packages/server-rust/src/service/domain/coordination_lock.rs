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

use dashmap::mapref::entry::Entry;
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
    ///
    /// # Errors
    /// Returns `LockError::InvalidLockName` if the name is empty, longer than
    /// 256 characters, or contains characters outside the allowed charset.
    pub fn validate_lock_name(name: &str) -> Result<(), LockError> {
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
    /// acquisition. Pattern ported from `TiKV`'s `check_txn_status.rs` — belt-and-suspenders against tokio timer
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

        // Atomic check-and-insert via DashMap Entry API: the shard guard is
        // held across the entire match, so two concurrent try_acquire calls
        // on the same name cannot both observe a vacant/expired slot and both
        // mint distinct fencing tokens. This preserves mutual exclusion
        // (invariant I1) under the multi-threaded tokio runtime.
        //
        // Reverse-index (`held_by`) and counter (`fencing_counters`) updates
        // target different DashMap instances, so we capture what we need from
        // inside the guard and apply those updates after the guard is released.
        let (fencing_token, stale_holder_to_evict) = match self.locks.entry(name.to_string()) {
            Entry::Occupied(mut occ) => {
                if occ.get().holder == holder {
                    // Invariant I3: same holder re-acquires — return the existing
                    // token unchanged (no new counter increment, no new timer).
                    return Ok(LockOutcome::Granted {
                        fencing_token: occ.get().fencing_token,
                    });
                }

                // Different holder: apply lazy TTL check (invariant I4).
                let is_expired = occ
                    .get()
                    .expires_at
                    .is_some_and(|exp| exp <= Instant::now());
                if !is_expired {
                    // Actively held by another holder.
                    return Ok(LockOutcome::Busy);
                }

                // Stale entry: evict in place while holding the guard. Abort the
                // old expiry task, remember the stale holder for reverse-index
                // cleanup below, then overwrite the entry with the new holder.
                let stale_holder = occ.get().holder;
                if let Some(task) = occ.get_mut().expiry_task.take() {
                    task.abort();
                }

                let new_token = self.next_fencing_token(name);
                let new_expires_at = ttl_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
                let new_expiry_task = self.spawn_expiry_task(name, new_token, ttl_ms);

                *occ.get_mut() = LockEntry {
                    holder,
                    fencing_token: new_token,
                    expires_at: new_expires_at,
                    expiry_task: new_expiry_task,
                };

                (new_token, Some(stale_holder))
            }
            Entry::Vacant(vac) => {
                let new_token = self.next_fencing_token(name);
                let new_expires_at = ttl_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
                let new_expiry_task = self.spawn_expiry_task(name, new_token, ttl_ms);

                vac.insert(LockEntry {
                    holder,
                    fencing_token: new_token,
                    expires_at: new_expires_at,
                    expiry_task: new_expiry_task,
                });

                (new_token, None)
            }
        };

        // Reverse-index maintenance (after the `locks` shard guard is released):
        // clean up stale holder's entry (if we evicted one) and add the new
        // holder's entry.
        if let Some(stale) = stale_holder_to_evict {
            if let Some(held) = self.held_by.get(&stale) {
                held.remove(name);
            }
        }
        self.held_by
            .entry(holder)
            .or_default()
            .insert(name.to_string());

        Ok(LockOutcome::Granted { fencing_token })
    }

    /// Mints the next strictly-monotonic fencing token for `name`.
    /// Separate method so the `try_acquire` guard sites can share the logic.
    fn next_fencing_token(&self, name: &str) -> u64 {
        let counter = self
            .fencing_counters
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        counter.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Spawns a TTL-driven expiry task for `(name, fencing_token)` if
    /// `ttl_ms` is `Some(ms)` with `ms > 0`. Returns `None` when no TTL is
    /// requested. Callers must ensure `ttl_ms != Some(0)` has already been
    /// rejected by `try_acquire`.
    fn spawn_expiry_task(
        self: &Arc<Self>,
        name: &str,
        fencing_token: u64,
        ttl_ms: Option<u64>,
    ) -> Option<JoinHandle<()>> {
        ttl_ms.map(|ms| {
            let registry = Arc::clone(self);
            let name_owned = name.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                registry.expire_if_token_matches(&name_owned, fencing_token);
            })
        })
    }

    /// Releases a lock if the fencing token matches the currently-held
    /// token. Returns `true` on successful release, `false` if the name
    /// is not held or the token does not match.
    ///
    /// Cancels any pending TTL expiry task.
    #[must_use]
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
    #[must_use]
    pub fn holder(&self, name: &str) -> Option<ConnectionId> {
        self.locks.get(name).map(|e| e.holder)
    }

    /// Returns the current fencing token for a lock, if held (for testing).
    #[must_use]
    pub fn current_token(&self, name: &str) -> Option<u64> {
        self.locks.get(name).map(|e| e.fencing_token)
    }

    /// Returns the number of active locks (for testing).
    #[must_use]
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
        let LockOutcome::Granted {
            fencing_token: token2,
        } = outcome2
        else {
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

    // -- Race condition and concurrent tests (G2b) --

    /// Verifies that concurrent disconnect and TTL expiry settle consistently
    /// without panic. The race: `release_on_disconnect` removes the entry while
    /// `expire_if_token_matches` is also trying to remove it. The token-match
    /// guard in `expire_if_token_matches` ensures the second remover sees
    /// nothing and no-ops safely.
    #[tokio::test]
    async fn disconnect_during_ttl_firing_is_safe() {
        // Run many iterations to expose any race condition under concurrent access.
        for _ in 0..50 {
            let reg = registry();
            let outcome = reg.try_acquire("lock-a", conn(1), Some(50)).unwrap();
            let LockOutcome::Granted { fencing_token } = outcome else {
                panic!("expected Granted");
            };

            let reg_clone = Arc::clone(&reg);
            // Spawn a task that calls release_on_disconnect concurrently while
            // the TTL timer may also be firing.
            let disconnect_task = tokio::spawn(async move {
                // Small delay to race with the timer.
                tokio::time::sleep(Duration::from_millis(30)).await;
                reg_clone.release_on_disconnect(conn(1));
            });

            // Also wait for the TTL to potentially fire.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = disconnect_task.await;

            // Either path may have removed the entry; no panic is the invariant.
            // After both paths complete, the lock must be gone.
            assert_eq!(
                reg.lock_count(),
                0,
                "no lock must remain after disconnect+TTL race"
            );
            // The fencing token guard ensures no double-free panic.
            let _ = fencing_token;
        }
    }

    /// Verifies that exactly one of N concurrent `try_acquire` calls wins.
    /// Spawns 100 tasks racing on the same lock name; asserts exactly one
    /// receives `Granted` (`DashMap` entry-level locking ensures mutual exclusion).
    ///
    /// Uses the multi-threaded tokio runtime and runs 50 iterations to expose
    /// any check-and-insert race on real multi-core hardware. With the
    /// current-thread runtime this test would pass even with a broken
    /// implementation because `try_acquire` has no `.await` points and tasks
    /// execute serially between polls.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_acquire_only_one_wins() {
        for _ in 0..50 {
            let reg = registry();
            let mut handles = Vec::new();

            for i in 0u64..100 {
                let reg_clone = Arc::clone(&reg);
                handles.push(tokio::spawn(async move {
                    reg_clone
                        .try_acquire("lock-a", ConnectionId(i), Some(10000))
                        .unwrap()
                }));
            }

            let results: Vec<LockOutcome> = futures_util::future::join_all(handles)
                .await
                .into_iter()
                .map(|r| r.expect("task panicked"))
                .collect();

            let granted_count = results
                .iter()
                .filter(|o| matches!(o, LockOutcome::Granted { .. }))
                .count();

            assert_eq!(
                granted_count, 1,
                "exactly one task must receive Granted, got {granted_count}"
            );
            assert_eq!(reg.lock_count(), 1);
        }
    }

    /// Verifies that interleaved concurrent release+acquire on the same lock
    /// name produces a consistent final state: exactly one holder or no holder.
    /// No panic must occur under contention.
    ///
    /// Uses the multi-threaded tokio runtime and runs 50 iterations so that
    /// interleaved `release`/`try_acquire` actually execute in parallel on
    /// different worker threads, exposing any check-and-insert race that a
    /// current-thread runtime would mask.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_acquire_and_release_is_safe() {
        for _ in 0..50 {
            let reg = registry();

            // Initial acquire.
            let outcome = reg.try_acquire("lock-a", conn(1), Some(10000)).unwrap();
            let LockOutcome::Granted {
                fencing_token: initial_token,
            } = outcome
            else {
                panic!("expected initial Granted");
            };

            let mut handles = Vec::new();

            // Mix of release and acquire tasks racing against each other.
            for i in 0u64..50 {
                let reg_rel = Arc::clone(&reg);
                let token = initial_token;
                handles.push(tokio::spawn(async move {
                    let _ = reg_rel.release("lock-a", token);
                }));

                let reg_acq = Arc::clone(&reg);
                handles.push(tokio::spawn(async move {
                    let _ = reg_acq.try_acquire("lock-a", ConnectionId(100 + i), Some(10000));
                }));
            }

            futures_util::future::join_all(handles)
                .await
                .into_iter()
                .for_each(|r| r.expect("task panicked"));

            // Final state must be consistent: 0 or 1 lock held.
            assert!(
                reg.lock_count() <= 1,
                "lock count must be 0 or 1 after concurrent access, got {}",
                reg.lock_count()
            );
        }
    }
}
