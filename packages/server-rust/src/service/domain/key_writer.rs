//! Per-KEY single-writer registry for the CRDT apply path.
//!
//! Provides `KeyWriterRegistry`, a `DashMap`-backed store of per-key
//! `tokio::sync::Mutex<()>` guards. Callers acquire the guard for a
//! `(map_name, key)` pair and hold it across the compound, `.await`-spanning
//! read-modify-write merge (`store.get` -> mutate -> `store.put`),
//! serializing concurrent writers on the SAME key without contending on
//! unrelated keys.
//!
//! # Why per-KEY, not per-partition
//!
//! A partition holds many unrelated keys. Serializing on the partition would
//! block concurrent writers to *different* keys that merely happen to hash
//! to the same partition — a throughput cliff with zero correctness
//! benefit, since the CRDT merge critical section is inherently scoped to
//! one key's record. Per-key locking gives exactly the exclusion the RMW
//! needs and nothing more.
//!
//! # Why `tokio::sync::Mutex`, not `std::sync::Mutex`
//!
//! The guard must be held across `.await` points (`store.get`/`store.put`
//! are async). Only `tokio::sync::Mutex`'s guard is `Send` and legal to hold
//! across an `.await`; a `std::sync::Mutex` guard held across `.await`
//! either fails to compile under the `Send` bound tokio's multi-threaded
//! executor requires on spawned futures, or risks real deadlock/starvation
//! if it did compile.
//!
//! # Non-duplication — this is not a rename of an existing mechanism
//!
//! Two existing per-key/token mechanisms were evaluated and are
//! insufficient for this job:
//! - `coordination_lock::LockRegistry` is a user-facing, named-lease
//!   distributed lock (`Granted`/fencing-token semantics for
//!   client-requested named locks, `try_acquire` returns immediately). It
//!   serializes *client lock requests*, not the server's internal CRDT
//!   apply path.
//! - The storage engine's `mark_stored` write-token identity check
//!   (`storage/engine.rs`, `storage/impls/default_record_store.rs`) guards a
//!   single `put` call against a stale writer overwriting a newer one — a
//!   point check on ONE write. It does not serialize the compound,
//!   `.await`-spanning `get -> modify -> put` sequence: two concurrent
//!   `OR_ADD`s can each pass their own `mark_stored` check on their own `put`
//!   while both having read the same pre-mutation state, producing the
//!   exact lost update this primitive exists to close.
//!
//! # Lock-map lifecycle (no eviction, by design)
//!
//! This registry never removes entries — the map only grows via
//! `or_insert_with`. Naive eviction here would be a mutual-exclusion
//! *correctness* bug, not merely a missed memory optimization: if task A
//! drops its guard and `remove`s the map entry while task B still holds a
//! clone of that same `Arc<Mutex>` (waiting on it), a later task C's
//! `or_insert_with` mints a *fresh* `Mutex` for the same key — B and C now
//! both believe they hold the key's lock, and mutual exclusion is broken. A
//! correct eviction would require `DashMap::remove_if(&key, |arc|
//! Arc::strong_count(arc) == 1)` executed atomically under the shard lock
//! against concurrent `or_insert_with` acquirers, and any background reaper
//! would need the identical guard (a two-step "iterate, check, then remove"
//! reaper is unsound). This child implements neither — unbounded growth,
//! bounded by the number of distinct keys ever written, is accepted as a
//! documented operational memory concern. Guarded eviction (option b) is
//! only adopted by a follow-up spec if key churn demonstrates it is needed.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Identifies a single CRDT record for per-key single-writer serialization.
///
/// Composed of `(map_name, key)` rather than the bare storage key alone:
/// keys are only unique *within* a map, and scoping the lock to the map
/// avoids false contention between unrelated maps whose keys happen to
/// share the same string value.
pub type WriterKey = (String, String);

/// Per-KEY single-writer registry.
///
/// Serializes the compound, `.await`-spanning read-modify-write merge
/// (`store.get` -> mutate -> `store.put`) used by CRDT apply, so concurrent
/// writers to the SAME key cannot interleave and lose an update. See the
/// module docs for the per-key-vs-per-partition rationale and the
/// no-eviction lifecycle contract.
pub struct KeyWriterRegistry {
    locks: DashMap<WriterKey, Arc<Mutex<()>>>,
}

impl KeyWriterRegistry {
    /// Creates a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            locks: DashMap::new(),
        }
    }

    /// Returns the per-key `Arc<Mutex<()>>` for `(map_name, key)`, inserting
    /// a fresh one if this is the first acquisition for the key. The
    /// registry NEVER removes entries — see module docs "Lock-map
    /// lifecycle".
    fn entry_lock(&self, map_name: &str, key: &str) -> Arc<Mutex<()>> {
        self.locks
            .entry((map_name.to_string(), key.to_string()))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Acquires the per-key writer lock for `(map_name, key)`, returning an
    /// owned RAII guard that can be held across `.await` points — including
    /// across a longer async scope than the acquisition call itself (per
    /// the module's `tokio::sync::Mutex` rationale). Callers span this
    /// guard over their entire critical region — for the `OR_ADD` joint-fix,
    /// from `store.get` through the single `store.put` merge-commit.
    pub async fn acquire(&self, map_name: &str, key: &str) -> KeyWriterGuard {
        let lock = self.entry_lock(map_name, key);
        let guard = lock.lock_owned().await;
        KeyWriterGuard { _guard: guard }
    }

    /// Number of distinct `(map_name, key)` pairs currently tracked. The
    /// registry only grows (no eviction — see module docs), so this value
    /// is monotonically non-decreasing over the registry's lifetime.
    /// Exposed for tests/inspection.
    #[must_use]
    pub fn tracked_key_count(&self) -> usize {
        self.locks.len()
    }
}

impl Default for KeyWriterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for a held per-key writer lock. Dropping it releases the
/// lock. Deliberately opaque (no exposed methods) — its only job is to keep
/// the per-key mutex held for the caller's critical-section lifetime; the
/// `OwnedMutexGuard` it wraps keeps the underlying `Arc<Mutex<()>>` alive
/// for as long as the guard is held, independent of the registry's own
/// lifetime.
pub struct KeyWriterGuard {
    _guard: OwnedMutexGuard<()>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    // -- AC6 (primitive half): the registry serializes concurrent
    //    acquisitions on the same key, closing a classic read-then-write
    //    lost-update race. --

    /// Simulates the exact RMW shape `crdt.rs`'s `OR_ADD` apply uses
    /// (read state -> yield across an await -> write state) on shared
    /// state that is UNPROTECTED except by the registry's per-key guard.
    /// Without correct mutual exclusion, concurrent read-then-write races
    /// would lose increments (the final count would be less than the
    /// number of tasks). With it, every task's increment survives.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_acquisitions_on_same_key_serialize_rmw_no_lost_update() {
        for _ in 0..20 {
            let registry = Arc::new(KeyWriterRegistry::new());
            let shared = Arc::new(AtomicU64::new(0));
            let n = 50u64;

            let mut handles = Vec::new();
            for _ in 0..n {
                let registry = Arc::clone(&registry);
                let shared = Arc::clone(&shared);
                handles.push(tokio::spawn(async move {
                    let _guard = registry.acquire("map", "same-key").await;
                    // Read-modify-write with a yield in between — this is the
                    // lost-update shape (read, await, write) rather than an
                    // atomic fetch_add, so it only converges under real
                    // mutual exclusion from the held guard.
                    let current = shared.load(Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    shared.store(current + 1, Ordering::SeqCst);
                }));
            }

            futures_util::future::join_all(handles)
                .await
                .into_iter()
                .for_each(|r| r.expect("task panicked"));

            assert_eq!(
                shared.load(Ordering::SeqCst),
                n,
                "per-key guard must serialize all {n} concurrent read-modify-writes on the \
                 same key with no lost update"
            );
        }
    }

    /// Control: acquisitions on DIFFERENT keys must not serialize against
    /// each other — this registry is per-KEY, not a single global lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_acquisitions_on_different_keys_do_not_block_each_other() {
        let registry = Arc::new(KeyWriterRegistry::new());

        // Hold key "a"'s lock for the duration of this scope.
        let _guard_a = registry.acquire("map", "a").await;

        // Acquiring an unrelated key "b" must complete promptly even while
        // "a" is held — proven by a bounded timeout rather than a hang.
        let registry_b = Arc::clone(&registry);
        let acquired_b = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::spawn(async move {
                let _guard_b = registry_b.acquire("map", "b").await;
            }),
        )
        .await;

        assert!(
            acquired_b.is_ok(),
            "acquiring a different key must not block on another key's held guard"
        );
    }

    // -- AC14: lock-map lifecycle — no eviction path exists; the registry
    //    only grows via `or_insert_with`, never shrinks. This is an accepted
    //    operational memory-growth concern (see module docs), not a bug. --

    #[tokio::test]
    async fn registry_never_evicts_entries_grows_monotonically() {
        let registry = KeyWriterRegistry::new();
        assert_eq!(registry.tracked_key_count(), 0);

        // Acquire and fully release a key's guard.
        {
            let _guard = registry.acquire("map", "k1").await;
        }
        assert_eq!(
            registry.tracked_key_count(),
            1,
            "the entry for k1 must remain tracked after its guard is dropped — no eviction"
        );

        // Re-acquiring the SAME key must reuse the existing entry, not add a
        // second one.
        {
            let _guard = registry.acquire("map", "k1").await;
        }
        assert_eq!(
            registry.tracked_key_count(),
            1,
            "re-acquiring the same key must not grow the tracked-key count"
        );

        // A distinct key gets its own tracked entry; the count only grows.
        {
            let _guard = registry.acquire("map", "k2").await;
        }
        assert_eq!(
            registry.tracked_key_count(),
            2,
            "a distinct key must be tracked as an additional entry (monotonic growth)"
        );
    }
}
