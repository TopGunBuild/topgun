//! Per-KEY single-writer registry for the CRDT apply path.
//!
//! Provides `KeyWriterRegistry`, a fixed table of `tokio::sync::Mutex<()>`
//! stripes. Callers acquire the guard for a `(map_name, key)` pair and hold it
//! across the compound, `.await`-spanning read-modify-write merge
//! (`store.get` -> mutate -> `store.put`), serializing concurrent writers on
//! the SAME key. A key's stripe is a hash of the pair, so two unrelated keys
//! usually do not contend, and when they land on one stripe they serialize
//! too.
//!
//! # Why per-KEY, not per-partition
//!
//! A partition holds many unrelated keys. Serializing on the partition would
//! block concurrent writers to *different* keys that merely happen to hash
//! to the same partition — a throughput cliff with zero correctness
//! benefit, since the CRDT merge critical section is inherently scoped to
//! one key's record. A stripe is the exclusion the RMW needs for its key plus
//! the few unrelated keys that hash to the same one of `KEY_WRITER_STRIPES`
//! entries: far finer than a partition, and never narrower than the key.
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
//! # The stripe table (fixed size, nothing per key)
//!
//! The table has exactly `KEY_WRITER_STRIPES` mutexes, all created at
//! construction. None is ever added, removed or replaced, and the registry
//! keeps nothing per key written, so its memory is bounded by
//! `KEY_WRITER_FOOTPRINT_BOUND_BYTES` however many distinct keys pass through
//! it. Because no entry is ever removed, a waiter can never be left holding a
//! mutex that a later caller no longer finds: every caller of a key reaches
//! the same mutex for the registry's whole life.
//!
//! A stripe's first use may allocate one small block for the platform's
//! mutex, once, and never again; every later acquire-and-release allocates
//! nothing. Measured: 64 B per stripe on macOS arm64. Linux: not measured.
//!
//! - **Same key, same stripe.** The stripe is a deterministic function of the
//!   two strings, so every writer of one `(map_name, key)` on one registry —
//!   whatever kind of write it is — excludes every other.
//! - **Different keys may share a stripe.** They then serialize as if they
//!   were one key. That is wider exclusion than correctness needs and is safe;
//!   it costs the waiter the rest of the holder's critical section.
//! - **A task holds at most one guard.** The mutex is not re-entrant, so a
//!   task that holds one key's guard and acquires another key blocks on itself
//!   whenever the two keys share a stripe. See `acquire`.
//!
//! # What a holder that never returns costs
//!
//! `acquire` has no bound of its own. A holder that never returns — a store
//! call that hangs under the guard — keeps its whole stripe, not only its key:
//!
//! - A client operation drawn to that stripe waits until the operation's own
//!   timeout drops it. A dispatch worker runs one operation at a time, so for
//!   that long the waiting operation occupies its worker and everything queued
//!   for that worker's partitions waits behind it. This repeats for each such
//!   operation that arrives.
//! - A caller that runs outside the timeout layer waits without bound. Two do
//!   so today: the tombstone prune task and the embedding write-back. While
//!   either waits, its whole pass stops, for every key.
//!
//! A new background caller must decide for itself how long it is willing to
//! wait.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedMutexGuard};

/// Number of stripes in a registry's table. A power of two, so a stripe index
/// is a mask of the key's hash.
///
/// Sized so that, with every dispatch worker and background writer permanently
/// inside a critical section, one operation finds its stripe held by an
/// unrelated writer less than 1 % of the time up to 64 workers.
pub const KEY_WRITER_STRIPES: usize = 16_384;

const _: () = assert!(KEY_WRITER_STRIPES.is_power_of_two());

/// Upper bound on the memory of one registry, before allocator overhead. It
/// covers the stripe table plus the one block per stripe that the platform's
/// mutex may allocate at the stripe's first use. The table never grows and
/// that block is allocated at most once per stripe, so this holds for the
/// registry's whole life.
pub const KEY_WRITER_FOOTPRINT_BOUND_BYTES: usize = 4 * 1024 * 1024;

/// The stripe of `(map, key)`.
///
/// The hashed bytes are the map name, one `0xFF` byte and the key. `0xFF`
/// never occurs in UTF-8, so the encoding of the pair is injective: `("ab",
/// "c")` and `("a", "bc")` are different inputs.
///
/// The hasher starts from fixed keys, so a pair has the same stripe in every
/// registry of a process. Which keys share a stripe may change with the
/// toolchain; nothing may depend on it.
fn stripe_index(map: &str, key: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    hasher.write(map.as_bytes());
    hasher.write_u8(0xFF);
    hasher.write(key.as_bytes());
    // Only the low bits survive the mask, so truncating the hash loses nothing.
    #[allow(clippy::cast_possible_truncation)]
    let hash = hasher.finish() as usize;
    hash & (KEY_WRITER_STRIPES - 1)
}

#[cfg(test)]
/// The stripe of `(map, key)`, for tests that need two keys on one stripe or
/// on two different ones.
pub(crate) fn stripe_of(map: &str, key: &str) -> usize {
    stripe_index(map, key)
}

/// Per-KEY single-writer registry.
///
/// Serializes the compound, `.await`-spanning read-modify-write merge
/// (`store.get` -> mutate -> `store.put`) used by CRDT apply, so concurrent
/// writers to the SAME key cannot interleave and lose an update. See the
/// module docs for the per-key-vs-per-partition rationale and the stripe
/// table.
pub struct KeyWriterRegistry {
    stripes: Box<[Arc<Mutex<()>>]>,
    #[cfg(test)]
    /// How many tasks are inside `acquire` and do not hold their guard yet.
    waiting: std::sync::atomic::AtomicUsize,
}

impl KeyWriterRegistry {
    /// Creates a registry with all `KEY_WRITER_STRIPES` stripes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stripes: (0..KEY_WRITER_STRIPES)
                .map(|_| Arc::new(Mutex::new(())))
                .collect(),
            #[cfg(test)]
            waiting: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Acquires the writer lock for `(map_name, key)`, returning an owned RAII
    /// guard that can be held across `.await` points — including across a
    /// longer async scope than the acquisition call itself (per the module's
    /// `tokio::sync::Mutex` rationale). Callers span this guard over their
    /// entire critical region: from the first read of the key's record that
    /// the write depends on through the last store call of that write.
    ///
    /// # Precondition
    ///
    /// A task must not call `acquire` while it holds a guard of this registry.
    /// Two different keys can share a stripe and the lock is not re-entrant,
    /// so the second call would wait for the guard its own task holds, and
    /// never return.
    pub async fn acquire<'k>(&self, map_name: &'k str, key: &'k str) -> KeyWriteToken<'k> {
        let lock = Arc::clone(&self.stripes[stripe_index(map_name, key)]);
        #[cfg(test)]
        let waiting = WaitingProbe::enter(&self.waiting);
        // Polled outside the task's cooperative budget. The runtime charges
        // every mutex acquire to that budget and sends the task back to the
        // run queue when it is spent, so a write path that takes a free key
        // per operation would give up its worker for a lock nobody holds. A
        // lock that IS held still returns `Pending` here and is woken by the
        // release: exclusion and the order of the waiters do not change.
        let guard = tokio::task::coop::unconstrained(lock.lock_owned()).await;
        #[cfg(test)]
        drop(waiting);
        KeyWriteToken {
            _guard: guard,
            map: map_name,
            key,
        }
    }
}

#[cfg(test)]
impl KeyWriterRegistry {
    /// How many tasks are waiting in `acquire` on this registry, whatever
    /// their key. A test may rely on the number only while the holder it
    /// waits behind is known to be parked and it has started no other writer
    /// on this registry: the counted writers then cannot leave `acquire`.
    pub(crate) fn test_waiting(&self) -> usize {
        self.waiting.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
/// Counts one task as waiting from just before it asks for the lock until it
/// has it. Leaving is done in `Drop`, so an `acquire` future dropped while it
/// still waits (a cancelled operation) never leaves the count raised.
struct WaitingProbe<'a> {
    waiting: &'a std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl<'a> WaitingProbe<'a> {
    fn enter(waiting: &'a std::sync::atomic::AtomicUsize) -> Self {
        waiting.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self { waiting }
    }
}

#[cfg(test)]
impl Drop for WaitingProbe<'_> {
    fn drop(&mut self) {
        self.waiting
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Default for KeyWriterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The held per-key writer of one `(map, key)`: the lock guard of the key's
/// stripe together with the pair it was acquired for.
///
/// The token IS the guard. The stripe stays locked exactly as long as the
/// token is alive and dropping the token is the only way to release it, so
/// the token cannot be cloned or copied and has no method that gives the
/// guard up. The `OwnedMutexGuard` it wraps keeps the underlying
/// `Arc<Mutex<()>>` alive for as long as the token is held, independent of
/// the registry's own lifetime.
///
/// The pair is borrowed from the arguments of `acquire`, so the token
/// allocates nothing. It carries the strings because a stripe does not
/// identify a key: two keys can share one, and the registry keeps no key at
/// all.
///
/// The token has no `Drop` impl of its own and must not get one; releasing
/// the lock is the guard field's drop. A destructor that could read the
/// borrowed strings would stretch their borrow to the end of the token's
/// scope, and a caller that moves the value it acquired from while the token
/// is still in scope — the tombstone prune pass does — would stop compiling.
pub struct KeyWriteToken<'k> {
    _guard: OwnedMutexGuard<()>,
    map: &'k str,
    key: &'k str,
}

impl KeyWriteToken<'_> {
    /// Whether the token was acquired for exactly `(map, key)`.
    ///
    /// Both strings are compared, never the stripe: a token of another key on
    /// the same stripe does hold the same lock, but accepting it would let a
    /// caller that locked the wrong key pass by a coincidence of the hash.
    #[must_use]
    pub fn covers(&self, map: &str, key: &str) -> bool {
        self.map == map && self.key == key
    }

    /// `Ok` when the token [`covers`](Self::covers) `(map, key)`. The token
    /// is only borrowed: checking does not give up the lock.
    ///
    /// # Errors
    ///
    /// [`KeyWriterMismatch`], naming the pair the token was acquired for and
    /// the pair it was asked about, when they differ.
    pub fn check(&self, map: &str, key: &str) -> Result<(), KeyWriterMismatch> {
        if self.covers(map, key) {
            return Ok(());
        }
        Err(KeyWriterMismatch {
            token_map: self.map.to_owned(),
            token_key: self.key.to_owned(),
            requested_map: map.to_owned(),
            requested_key: key.to_owned(),
        })
    }
}

#[cfg(test)]
impl<'k> KeyWriteToken<'k> {
    /// A token for `(map, key)` around the guard of a fresh mutex that
    /// nothing else can reach, for a test that needs a token and no registry.
    /// It is a real guard of a real lock, but it excludes nobody.
    pub(crate) fn for_test(map: &'k str, key: &'k str) -> Self {
        let guard = Arc::new(Mutex::new(()))
            .try_lock_owned()
            .expect("a mutex nobody else can reach is free");
        Self {
            _guard: guard,
            map,
            key,
        }
    }
}

/// A [`KeyWriteToken`] was asked about a `(map, key)` it was not acquired
/// for.
///
/// Only [`KeyWriteToken::check`] builds one, so a value of this type always
/// reports a mismatch that was observed. The strings are owned, which lets
/// the error travel through `anyhow` and be found again by `downcast_ref`;
/// they are allocated only when a mismatch is reported.
#[derive(Debug)]
pub struct KeyWriterMismatch {
    token_map: String,
    token_key: String,
    requested_map: String,
    requested_key: String,
}

impl KeyWriterMismatch {
    /// The map the token was acquired for.
    #[must_use]
    pub fn token_map(&self) -> &str {
        &self.token_map
    }

    /// The key the token was acquired for.
    #[must_use]
    pub fn token_key(&self) -> &str {
        &self.token_key
    }

    /// The map the token was asked about.
    #[must_use]
    pub fn requested_map(&self) -> &str {
        &self.requested_map
    }

    /// The key the token was asked about.
    #[must_use]
    pub fn requested_key(&self) -> &str {
        &self.requested_key
    }
}

impl std::fmt::Display for KeyWriterMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the key writer token of ({:?}, {:?}) was presented for ({:?}, {:?})",
            self.token_map, self.token_key, self.requested_map, self.requested_key
        )
    }
}

impl std::error::Error for KeyWriterMismatch {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::task::Poll;
    use std::time::Duration;

    use super::*;

    const MAP: &str = "map";
    /// How long a test waits for a point it must reach before it declares
    /// itself broken. A failure detector: no outcome depends on it.
    const WAIT_BOUND: Duration = Duration::from_secs(5);

    /// The first `k{i}` whose stripe relates to `reference`'s as asked: the
    /// same stripe, or a different one. `reference` itself is never returned.
    fn key_by_stripe(reference: &str, same_stripe: bool) -> String {
        let wanted = stripe_of(MAP, reference);
        (0..64 * KEY_WRITER_STRIPES)
            .map(|i| format!("k{i}"))
            .find(|candidate| {
                candidate != reference && (stripe_of(MAP, candidate) == wanted) == same_stripe
            })
            .expect("no candidate key with the wanted stripe")
    }

    /// The stripe table's memory, from the sizes of its parts: per stripe, the
    /// pointer in the table and the heap block of two reference counts and the
    /// mutex. Allocator overhead per block is not counted.
    fn stripe_table_bytes() -> usize {
        let pointer = std::mem::size_of::<Arc<Mutex<()>>>();
        let reference_counts = 2 * std::mem::size_of::<usize>();
        let mutex = std::mem::size_of::<Mutex<()>>();
        KEY_WRITER_STRIPES * (pointer + reference_counts + mutex)
    }

    /// Polls `future` exactly once and says whether that poll was `Pending`.
    async fn pending_on_first_poll<F: Future>(future: std::pin::Pin<&mut F>) -> bool {
        let mut future = future;
        std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
    }

    // -- The registry serializes concurrent acquisitions on the same key,
    //    closing a classic read-then-write lost-update race. --

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

    /// Control: acquisitions of keys on different stripes must not serialize
    /// against each other — this registry is not a single global lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_acquisitions_on_different_keys_do_not_block_each_other() {
        let registry = Arc::new(KeyWriterRegistry::new());
        let other = key_by_stripe("a", false);

        // Hold key "a"'s lock for the duration of this scope.
        let _guard_a = registry.acquire(MAP, "a").await;

        // Acquiring a key on a different stripe must complete promptly even
        // while "a" is held — proven by a bounded timeout rather than a hang.
        let registry_b = Arc::clone(&registry);
        let acquired_b = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::spawn(async move {
                let _guard_b = registry_b.acquire(MAP, &other).await;
            }),
        )
        .await;

        assert!(
            acquired_b.is_ok(),
            "acquiring a key on a different stripe must not block on another key's held guard"
        );
    }

    /// Two different keys that share a stripe exclude each other, and neither
    /// is left behind: the waiter gets the stripe when the holder releases it,
    /// and writers alternating between the two keys all complete.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_different_keys_on_one_stripe_both_complete() {
        const A: &str = "a";
        const STABLE_WAIT_YIELDS: usize = 200;
        let b = key_by_stripe(A, true);
        let registry = Arc::new(KeyWriterRegistry::new());

        let held_a = registry.acquire(MAP, A).await;
        let b_done = Arc::new(AtomicBool::new(false));
        let task = {
            let (registry, b, b_done) = (Arc::clone(&registry), b.clone(), Arc::clone(&b_done));
            tokio::spawn(async move {
                let _guard = registry.acquire(MAP, &b).await;
                b_done.store(true, Ordering::SeqCst);
            })
        };
        // B waits behind A only if the two share a mutex. One sighting of the
        // count is also true for a task passing through a free lock, so the
        // wait must still be there at each of the following yields.
        let b_waited_while_a_was_held = tokio::time::timeout(WAIT_BOUND, async {
            let mut sightings_after_first = 0;
            loop {
                if task.is_finished() {
                    return false;
                }
                if registry.test_waiting() == 1 {
                    if sightings_after_first == STABLE_WAIT_YIELDS {
                        return true;
                    }
                    sightings_after_first += 1;
                } else {
                    sightings_after_first = 0;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the acquire of B neither returned nor waited");

        drop(held_a);
        tokio::time::timeout(WAIT_BOUND, task)
            .await
            .expect("B never got the stripe after A released it")
            .expect("B task");
        let b_completed = b_done.load(Ordering::SeqCst);

        // One after the other: holding both at once is what a task must not do.
        let a_and_b_reacquired = tokio::time::timeout(WAIT_BOUND, async {
            drop(registry.acquire(MAP, A).await);
            drop(registry.acquire(MAP, &b).await);
        })
        .await
        .is_ok();

        assert_eq!(
            (b_waited_while_a_was_held, b_completed, a_and_b_reacquired),
            (true, true, true),
            "(B waited while A was held, B completed after the release, A and B can be \
             acquired again)"
        );

        // No starvation: writers alternating between the two keys share one
        // unprotected counter, and every one of them gets its turn.
        let shared = Arc::new(AtomicU64::new(0));
        let n = 50u64;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let (registry, shared) = (Arc::clone(&registry), Arc::clone(&shared));
                let key = if i % 2 == 0 { A.to_string() } else { b.clone() };
                tokio::spawn(async move {
                    let _guard = registry.acquire(MAP, &key).await;
                    let current = shared.load(Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    shared.store(current + 1, Ordering::SeqCst);
                })
            })
            .collect();
        tokio::time::timeout(WAIT_BOUND, futures_util::future::join_all(handles))
            .await
            .expect("a writer on the shared stripe never got its turn")
            .into_iter()
            .for_each(|r| r.expect("task panicked"));
        assert_eq!(
            shared.load(Ordering::SeqCst),
            n,
            "two keys on one stripe must serialize all {n} read-modify-writes with no lost update"
        );
    }

    /// Writing many distinct keys leaves nothing behind in the registry: the
    /// table has its fixed size, no stripe is still referenced by an acquire
    /// that has returned its guard, and no stripe is still locked. The keys
    /// must really have spread over the table for that to say anything.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn registry_holds_no_per_key_state_after_100_000_distinct_keys() {
        const KEYS: usize = 100_000;
        const TASKS: usize = 8;
        const MAPS: usize = 4;
        let registry = Arc::new(KeyWriterRegistry::new());

        let handles: Vec<_> = (0..TASKS)
            .map(|task| {
                let registry = Arc::clone(&registry);
                tokio::spawn(async move {
                    for i in (task..KEYS).step_by(TASKS) {
                        let _guard = registry
                            .acquire(&format!("m{}", i % MAPS), &format!("k{i}"))
                            .await;
                    }
                })
            })
            .collect();
        futures_util::future::join_all(handles)
            .await
            .into_iter()
            .for_each(|r| r.expect("task panicked"));
        assert_eq!(
            registry.test_waiting(),
            0,
            "quiescence: every task has joined, so none is inside acquire"
        );

        let stripes_with_a_live_clone = registry
            .stripes
            .iter()
            .filter(|stripe| Arc::strong_count(stripe) > 1)
            .count();
        let stripes_that_cannot_be_locked = registry
            .stripes
            .iter()
            .filter(|stripe| stripe.try_lock().is_err())
            .count();
        let distinct_stripes_touched = (0..KEYS)
            .map(|i| stripe_of(&format!("m{}", i % MAPS), &format!("k{i}")))
            .collect::<HashSet<_>>()
            .len();
        println!(
            "key_writer/no_per_key_state: keys={KEYS} stripe_count={} \
             stripes_with_a_live_clone={stripes_with_a_live_clone} \
             stripes_that_cannot_be_locked={stripes_that_cannot_be_locked} \
             distinct_stripes_touched={distinct_stripes_touched}",
            registry.stripes.len()
        );
        assert_eq!(
            (
                registry.stripes.len(),
                stripes_with_a_live_clone,
                stripes_that_cannot_be_locked,
                distinct_stripes_touched >= KEY_WRITER_STRIPES / 2
            ),
            (KEY_WRITER_STRIPES, 0, 0, true),
            "(stripe count, stripes still referenced outside the table, stripes still locked, \
             the keys spread over at least half the table)"
        );
    }

    /// The table's memory, computed from the sizes of its parts, is within the
    /// stated constant, and the registry has no room for anything but the
    /// table and the test counter. Allocator overhead per block is not counted.
    #[test]
    fn stripe_table_footprint_is_within_the_stated_constant() {
        let mutex = std::mem::size_of::<Mutex<()>>();
        let table = stripe_table_bytes();
        println!(
            "key_writer/footprint: size_of::<Mutex<()>>()={mutex} per_stripe={} table_bytes={table} \
             bound_bytes={KEY_WRITER_FOOTPRINT_BOUND_BYTES} size_of::<KeyWriterRegistry>()={}",
            table / KEY_WRITER_STRIPES,
            std::mem::size_of::<KeyWriterRegistry>()
        );
        assert!(
            table <= KEY_WRITER_FOOTPRINT_BOUND_BYTES,
            "the stripe table ({table} B) must fit the stated bound"
        );
        assert_eq!(
            std::mem::size_of::<KeyWriterRegistry>(),
            std::mem::size_of::<Box<[Arc<Mutex<()>>]>>() + std::mem::size_of::<AtomicUsize>(),
            "the registry holds the boxed stripe table and the test counter, nothing else"
        );
    }

    /// The registry allocates nothing per key and nothing per acquire. A cold
    /// pass over many distinct keys may allocate one block per stripe it
    /// touches (the platform's mutex, at the stripe's first use), and that
    /// plus the table stays within the stated bound; a second pass over the
    /// same keys allocates nothing.
    ///
    /// A local allocation proof, not a CI guard: CI never enables `count-alloc`,
    /// and the counters are process-global, so it is meaningful only when run
    /// alone and single-threaded:
    /// `cargo test --release -p topgun-server --lib --features count-alloc -- --ignored --test-threads=1 count_alloc_acquire`
    #[cfg(feature = "count-alloc")]
    #[test]
    #[ignore = "local allocation proof: run single-threaded under count-alloc"]
    fn count_alloc_acquire_is_bounded_on_first_use_and_zero_on_repeat() {
        const KEYS: usize = 100_000;
        let allocated = || stats_alloc::INSTRUMENTED_SYSTEM.stats().bytes_allocated;

        let registry = KeyWriterRegistry::new();
        let keys: Vec<String> = (0..KEYS).map(|i| format!("k{i}")).collect();
        let stripes_touched = keys
            .iter()
            .map(|key| stripe_of(MAP, key))
            .collect::<HashSet<_>>()
            .len();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime");

        // The counter is live: one formatted string moves it.
        let before = allocated();
        let formatted = std::hint::black_box(format!("k{KEYS}"));
        let one_format = allocated() - before;
        drop(formatted);

        // Both passes run inside one `block_on`. The runtime allocates one
        // block of its own per `block_on` while the loop runs (seen as 64 B
        // beyond the stripes' blocks in the cold figure); in a second
        // `block_on` that block would be charged to the repeat pass.
        let (cold, repeat) = runtime.block_on(async {
            let before = allocated();
            for key in &keys {
                drop(registry.acquire(MAP, key).await);
            }
            let after_cold = allocated();
            for key in &keys {
                drop(registry.acquire(MAP, key).await);
            }
            (after_cold - before, allocated() - after_cold)
        });

        let table = stripe_table_bytes();
        println!(
            "count_alloc_acquire acquires_per_pass={KEYS} stripes_touched={stripes_touched} \
             cold_pass_bytes_allocated={cold} cold_bytes_per_touched_stripe={} \
             cold_bytes_remainder={} repeat_pass_bytes_allocated={repeat} table_bytes={table} \
             bound_bytes={KEY_WRITER_FOOTPRINT_BOUND_BYTES} one_format_bytes_allocated={one_format}",
            cold / stripes_touched,
            cold % stripes_touched
        );
        assert!(one_format > 0, "the allocation counter must be live");
        assert!(
            cold + table <= KEY_WRITER_FOOTPRINT_BOUND_BYTES,
            "the first use of {stripes_touched} stripes ({cold} B) plus the table ({table} B) \
             must fit the stated bound"
        );
        assert_eq!(
            repeat, 0,
            "{KEYS} acquire-and-release calls on stripes already used must allocate nothing"
        );
    }

    /// Counts the polls of the future it wraps.
    struct CountPolls<F> {
        future: std::pin::Pin<Box<F>>,
        polls: Arc<AtomicUsize>,
    }

    impl<F: Future> Future for CountPolls<F> {
        type Output = F::Output;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<F::Output> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.future.as_mut().poll(cx)
        }
    }

    /// An acquire of a free key does not make its task yield. The task below
    /// awaits nothing but acquires of keys nobody holds, more of them than
    /// the runtime lets one poll of a task complete when each is charged to
    /// the task's cooperative budget; charged, the task is sent back to the
    /// run queue part-way and polled again, and the count below is above one.
    /// A write path that acquires a free key per operation would hand its
    /// worker back to the scheduler that often for a lock nobody contends.
    #[tokio::test]
    async fn acquires_of_free_keys_complete_in_one_poll_of_their_task() {
        const ACQUIRES: usize = 1_024;
        let registry = KeyWriterRegistry::new();
        let keys: Vec<String> = (0..ACQUIRES).map(|i| format!("k{i}")).collect();
        let polls = Arc::new(AtomicUsize::new(0));

        let task = tokio::spawn(CountPolls {
            future: Box::pin(async move {
                let mut acquired = 0;
                for key in &keys {
                    drop(registry.acquire(MAP, key).await);
                    acquired += 1;
                }
                acquired
            }),
            polls: Arc::clone(&polls),
        });
        let acquired = tokio::time::timeout(WAIT_BOUND, task)
            .await
            .expect("the acquiring task never finished")
            .expect("acquiring task");

        assert_eq!(
            (acquired, polls.load(Ordering::SeqCst)),
            (ACQUIRES, 1),
            "(acquires of free keys made, polls of the task that made them)"
        );
    }

    /// What the one-guard-per-task precondition prevents: a task that holds a
    /// key and acquires another key on the same stripe waits for itself. The
    /// second acquire is polled once and dropped, which also shows that a
    /// dropped acquire leaves the waiting count.
    #[tokio::test]
    async fn a_second_acquire_by_the_holding_task_on_the_same_stripe_stays_pending() {
        const A: &str = "a";
        let same_stripe = key_by_stripe(A, true);
        let other_stripe = key_by_stripe(A, false);
        let registry = KeyWriterRegistry::new();
        let _held_a = registry.acquire(MAP, A).await;

        // Non-vacuity: a single poll does complete an acquire that is free.
        let other_is_pending = {
            let acquire = std::pin::pin!(registry.acquire(MAP, &other_stripe));
            pending_on_first_poll(acquire).await
        };
        assert!(
            !other_is_pending,
            "precondition: with A held, a key on a different stripe is acquired at the first poll"
        );

        let (is_pending, waiting_while_polled) = {
            let acquire = std::pin::pin!(registry.acquire(MAP, &same_stripe));
            let is_pending = pending_on_first_poll(acquire).await;
            (is_pending, registry.test_waiting())
        };
        let waiting_after_the_future_was_dropped = registry.test_waiting();

        assert_eq!(
            (
                is_pending,
                waiting_while_polled,
                waiting_after_the_future_was_dropped
            ),
            (true, 1, 0),
            "(the second acquire is pending, it is counted as waiting while it lives, the count \
             is back to zero once it is dropped)"
        );
    }

    // -- The token: it knows its key, and it is the lock guard. --

    // The token is the only handle on its lock, so a copy of it would be a
    // second handle that outlives the first. The guard field already rules
    // out a derived `Clone`; this fails the test build if a later change of
    // the field's type, or a hand-written impl, makes the token clonable.
    static_assertions::assert_not_impl_any!(KeyWriteToken<'static>: Clone, Copy);

    /// A token answers for the pair it was acquired for and for no other —
    /// not even a key that shares its stripe, whose lock it does hold — and
    /// while it is alive the key's writer is taken.
    #[tokio::test]
    async fn a_token_covers_exactly_the_key_it_was_acquired_for() {
        const A: &str = "a";
        const OTHER_MAP: &str = "other-map";
        let same_stripe = key_by_stripe(A, true);
        let other_stripe = key_by_stripe(A, false);
        let registry = KeyWriterRegistry::new();
        let token = registry.acquire(MAP, A).await;

        // Non-vacuity of the fourth element: the refused key is on the
        // token's own stripe.
        assert_eq!(
            stripe_of(MAP, &same_stripe),
            stripe_of(MAP, A),
            "precondition: the same-stripe key shares the token's stripe"
        );
        assert_eq!(
            (
                token.covers(MAP, A),
                token.covers(MAP, &other_stripe),
                token.covers(OTHER_MAP, A),
                token.covers(MAP, &same_stripe),
            ),
            (true, false, false, false),
            "(its own map and key, another key, another map, another key on its own stripe)"
        );

        // One sighting of the waiting count is also true for a task passing
        // through a free lock, so the wait is taken from the first poll of
        // the second acquire; the count is read while that future lives.
        let mut second = std::pin::pin!(registry.acquire(MAP, A));
        let pending_while_the_token_lives = pending_on_first_poll(second.as_mut()).await;
        let waiting_while_the_token_lives = registry.test_waiting();
        assert_eq!(
            (pending_while_the_token_lives, waiting_while_the_token_lives),
            (true, 1),
            "(a second acquire of the same key is pending at its first poll, it is counted as \
             waiting)"
        );

        drop(token);
        let second_token = tokio::time::timeout(WAIT_BOUND, second)
            .await
            .expect("the second acquire never completed after the token was dropped");
        assert!(second_token.covers(MAP, A));
    }

    /// `check` is `Ok` for the token's own pair and otherwise names both
    /// pairs, and the error is still found after a trip through `anyhow`.
    #[test]
    fn check_names_both_identities_on_a_mismatch_and_is_ok_on_a_match() {
        fn identities(mismatch: &KeyWriterMismatch) -> (&str, &str, &str, &str) {
            (
                mismatch.token_map(),
                mismatch.token_key(),
                mismatch.requested_map(),
                mismatch.requested_key(),
            )
        }
        let token = KeyWriteToken::for_test("m", "k");

        let matching = token.check("m", "k").is_ok();
        let wrong_key = token
            .check("m", "other-key")
            .expect_err("a wrong key is a mismatch");
        let wrong_map = token
            .check("other-map", "k")
            .expect_err("a wrong map is a mismatch");
        let through_anyhow = anyhow::Error::from(
            token
                .check("m", "other-key")
                .expect_err("a wrong key is a mismatch"),
        );
        let found_again = through_anyhow
            .downcast_ref::<KeyWriterMismatch>()
            .map(identities);

        assert_eq!(
            (
                matching,
                identities(&wrong_key),
                identities(&wrong_map),
                found_again,
            ),
            (
                true,
                ("m", "k", "m", "other-key"),
                ("m", "k", "other-map", "k"),
                Some(("m", "k", "m", "other-key")),
            ),
            "(the token's own pair is Ok, a wrong key, a wrong map, the wrong-key error found \
             again behind anyhow) — each mismatch as (token map, token key, requested map, \
             requested key)"
        );
    }
}
