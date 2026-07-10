//! Write-behind buffering layer for [`MapDataStore`].
//!
//! Decouples mutation latency from persistence latency by buffering writes
//! in per-partition coalesced queues and flushing them on a configurable schedule.
//! An optional WAL ensures every acked write is durable before the ack is returned.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{watch, Notify};
use topgun_core::{fnv1a_hash, PARTITION_COUNT};
use tracing::warn;

use crate::storage::map_data_store::{
    merkle_leaf_hash, LeafSink, MapDataStore, MerkleLeaf, ScanBatch, ScanCursor,
};
use crate::storage::record::RecordValue;
use crate::storage::wal::{Wal, WalEntry, WalFsyncPolicy, WalOp, WalStorePayload};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the write-behind buffering layer.
///
/// Controls flush timing, batch sizes, retry behavior, capacity limits, and
/// the maximum time to wait for a graceful-shutdown drain before giving up.
/// WAL fields control crash-safe durability for acked writes.
#[derive(Debug, Clone)]
pub struct WriteBehindConfig {
    /// Delay in milliseconds before a queued entry becomes eligible for flush.
    pub write_delay_ms: u64,
    /// How often the background flush task runs, in milliseconds.
    pub flush_interval_ms: u64,
    /// Maximum number of entries to process per flush batch.
    pub batch_size: u32,
    /// Number of retry attempts for failed flush operations.
    pub max_retries: u32,
    /// Initial retry backoff in milliseconds (doubles on each retry).
    pub backoff_base_ms: u64,
    /// Maximum retry backoff cap in milliseconds.
    pub backoff_cap_ms: u64,
    /// Node-wide maximum pending entries. 0 means unlimited.
    pub capacity: u64,
    /// Maximum milliseconds to wait for the write-behind buffer to drain on
    /// graceful shutdown. On timeout, still-pending ops are logged and the
    /// process exits rather than blocking termination indefinitely.
    pub shutdown_timeout_ms: u64,
    /// Directory for WAL files. Must be on the same volume as the durable store
    /// to avoid split-brain on partial-disk failure.
    pub wal_dir: PathBuf,
    /// Controls how aggressively the WAL writer calls fsync after writing frames.
    ///
    /// The default is `Batched`: a write is acked after its frame is appended, but
    /// the fsync is deferred to a ~10ms group-commit timer (or a 100-frame flush).
    /// This is the throughput-favouring default — it does NOT make an acked write
    /// durable under an unclean `kill -9`: writes inside the group-commit window are
    /// lost. That is an accepted tradeoff for the single-node demo tier, where the
    /// originating client still holds the write locally and re-converges on the next
    /// delta-sync. Set `TOPGUN_WAL_FSYNC_POLICY=per_op` to make every acked write
    /// fsynced-before-ack (acked == durable) at a large per-write latency cost.
    pub wal_fsync_policy: WalFsyncPolicy,
}

impl Default for WriteBehindConfig {
    fn default() -> Self {
        Self {
            write_delay_ms: 1000,
            flush_interval_ms: 1000,
            batch_size: 100,
            max_retries: 3,
            backoff_base_ms: 500,
            backoff_cap_ms: 5000,
            capacity: 10_000,
            shutdown_timeout_ms: 30_000,
            wal_dir: PathBuf::from("./topgun-wal"),
            wal_fsync_policy: WalFsyncPolicy::Batched,
        }
    }
}

impl WriteBehindConfig {
    /// Construct [`WriteBehindConfig`] from environment variables.
    ///
    /// Reads env vars; any missing or unparseable var falls back to the
    /// corresponding [`Self::default`] field and emits a `tracing::warn!`. The
    /// server never panics due to a misconfigured write-behind env var.
    ///
    /// | Env var | Field | Default |
    /// |---|---|---|
    /// | `TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS` | `flush_interval_ms` | 1000 ms |
    /// | `TOPGUN_WRITEBEHIND_BATCH_SIZE` | `batch_size` | 100 |
    /// | `TOPGUN_WRITEBEHIND_CAPACITY` | `capacity` | 10 000 |
    /// | `TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS` | `shutdown_timeout_ms` | 30 000 ms |
    /// | `TOPGUN_WAL_DIR` | `wal_dir` | `./topgun-wal` |
    /// | `TOPGUN_WAL_FSYNC_POLICY` | `wal_fsync_policy` | `batched` |
    ///
    /// Fields not covered by env vars (`write_delay_ms`, `max_retries`,
    /// `backoff_base_ms`, `backoff_cap_ms`) retain their [`Self::default`] values.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// Construct [`WriteBehindConfig`] from an injected variable source.
    ///
    /// `get` maps a variable name to its value (or `None` if unset). This is the
    /// testable seam behind [`Self::from_env`], which simply passes a closure
    /// over `std::env::var`. Tests pass a map-backed closure so they exercise the
    /// full parse logic without mutating process-global environment state —
    /// eliminating the cross-test env race that made parallel runs flaky.
    ///
    /// # Panics
    ///
    /// Panics (refuses to start) if `TOPGUN_WAL_FSYNC_POLICY` is set to a value
    /// that does not parse to a known [`WalFsyncPolicy`]. Silently downgrading an
    /// unknown durability policy to the weaker default is what masked a durability
    /// regression through a full RED soak, so the misconfiguration is made fatal at
    /// startup rather than surfacing as data loss on the next unclean shutdown.
    #[must_use]
    pub fn from_source<F: Fn(&str) -> Option<String>>(get: F) -> Self {
        let defaults = Self::default();
        let mut cfg = defaults.clone();

        // Parse TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS → flush_interval_ms (u64)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS") {
            match raw.trim().parse::<u64>() {
                Ok(ms) => {
                    cfg.flush_interval_ms = ms;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS",
                        value = %raw,
                        error = %err,
                        default = defaults.flush_interval_ms,
                        "Failed to parse env var; using default"
                    );
                    cfg.flush_interval_ms = defaults.flush_interval_ms;
                }
            }
        }

        // Parse TOPGUN_WRITEBEHIND_BATCH_SIZE → batch_size (u32)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_BATCH_SIZE") {
            match raw.trim().parse::<u32>() {
                Ok(size) => {
                    cfg.batch_size = size;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_BATCH_SIZE",
                        value = %raw,
                        error = %err,
                        default = defaults.batch_size,
                        "Failed to parse env var; using default"
                    );
                    cfg.batch_size = defaults.batch_size;
                }
            }
        }

        // Parse TOPGUN_WRITEBEHIND_CAPACITY → capacity (u64)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_CAPACITY") {
            match raw.trim().parse::<u64>() {
                Ok(cap) => {
                    cfg.capacity = cap;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_CAPACITY",
                        value = %raw,
                        error = %err,
                        default = defaults.capacity,
                        "Failed to parse env var; using default"
                    );
                    cfg.capacity = defaults.capacity;
                }
            }
        }

        // Parse TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS → shutdown_timeout_ms (u64)
        if let Some(raw) = get("TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS") {
            match raw.trim().parse::<u64>() {
                Ok(ms) => {
                    cfg.shutdown_timeout_ms = ms;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS",
                        value = %raw,
                        error = %err,
                        default = defaults.shutdown_timeout_ms,
                        "Failed to parse env var; using default"
                    );
                    cfg.shutdown_timeout_ms = defaults.shutdown_timeout_ms;
                }
            }
        }

        // Parse TOPGUN_WAL_DIR → wal_dir (PathBuf)
        if let Some(raw) = get("TOPGUN_WAL_DIR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                cfg.wal_dir = PathBuf::from(trimmed);
            }
        }

        // Parse TOPGUN_WAL_FSYNC_POLICY → wal_fsync_policy (WalFsyncPolicy)
        //
        // An unknown value is FATAL: silently falling back to the Batched default
        // here is exactly what masked a durability regression through a full RED
        // soak (a misspelled durable policy quietly ran with the weaker default).
        // Refusing to start forces the misconfiguration to surface at boot rather
        // than as silent data loss on the next unclean shutdown.
        if let Some(raw) = get("TOPGUN_WAL_FSYNC_POLICY") {
            match raw.trim().parse::<WalFsyncPolicy>() {
                Ok(policy) => {
                    cfg.wal_fsync_policy = policy;
                }
                Err(err) => {
                    panic!(
                        "TOPGUN_WAL_FSYNC_POLICY={raw:?} is not a valid WAL fsync policy: {err}. \
                         Refusing to start: an unknown durability policy must not be silently \
                         downgraded to a weaker default. Set a valid value or unset the variable."
                    );
                }
            }
        }

        cfg
    }
}

// ---------------------------------------------------------------------------
// Delayed operation types
// ---------------------------------------------------------------------------

/// A buffered operation waiting to be flushed to the inner store.
#[derive(Debug, Clone)]
pub(crate) enum DelayedOp {
    /// Buffered add/update with the value and expiration time.
    Store {
        value: RecordValue,
        expiration_time: i64,
    },
    /// Buffered remove (tombstone).
    Remove,
}

/// An entry in the write-behind queue representing a pending operation.
#[derive(Debug, Clone)]
pub(crate) struct DelayedEntry {
    /// Map name this entry belongs to.
    pub map: String,
    /// Record key within the map.
    pub key: String,
    /// The buffered operation (store or remove).
    pub operation: DelayedOp,
    /// Wall-clock millis when first enqueued. Preserved on coalesce so the
    /// key flushes on its original schedule.
    pub store_time: i64,
    /// Global ordering counter for fairness during flush.
    pub sequence: u64,
    /// Number of failed flush attempts so far.
    pub retry_count: u32,
    /// WAL sequence number for this entry; used to mark it applied after flush.
    /// Zero means no WAL entry was written (e.g., when WAL is disabled).
    pub wal_sequence: u64,
}

// ---------------------------------------------------------------------------
// Partition queue
// ---------------------------------------------------------------------------

/// Per-partition queue that coalesces writes by (map, key).
///
/// Only the latest value per key is stored; frequently-updated keys
/// do not starve the queue.
#[derive(Debug, Default)]
pub(crate) struct PartitionQueue {
    entries: HashMap<(String, String), DelayedEntry>,
}

impl PartitionQueue {
    /// Inserts or replaces an entry for the given (map, key).
    ///
    /// Returns the previous entry if one was replaced (coalesce),
    /// or `None` if this was a new key.
    pub fn insert(&mut self, entry: DelayedEntry) -> Option<DelayedEntry> {
        let key = (entry.map.clone(), entry.key.clone());
        self.entries.insert(key, entry)
    }

    /// Removes and returns the entry for the given key, or `None` if not present.
    pub fn remove(&mut self, map: &str, key: &str) -> Option<DelayedEntry> {
        self.entries.remove(&(map.to_string(), key.to_string()))
    }

    /// Collects all entries whose `store_time` is at or before the deadline.
    ///
    /// Eligible entries are removed from the queue and returned sorted by
    /// `store_time` then `sequence` for fairness.
    pub fn drain_ready(&mut self, deadline: i64) -> Vec<DelayedEntry> {
        let ready_keys: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.store_time <= deadline)
            .map(|(k, _)| k.clone())
            .collect();

        let mut ready = Vec::with_capacity(ready_keys.len());
        for k in ready_keys {
            if let Some(entry) = self.entries.remove(&k) {
                ready.push(entry);
            }
        }
        ready
    }

    /// Re-inserts entries that failed flush and need retry.
    ///
    /// Preserves original `store_time` so the entry retains its flush priority.
    pub fn reinsert_front(&mut self, entries: Vec<DelayedEntry>) {
        for entry in entries {
            let key = (entry.map.clone(), entry.key.clone());
            // Only reinsert if no newer entry exists for this key
            self.entries.entry(key).or_insert(entry);
        }
    }

    /// Returns the number of entries in this partition queue.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// Partition assignment
// ---------------------------------------------------------------------------

/// Current wall-clock time as millis since epoch.
fn now_millis() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// Deterministic partition assignment from (map, key) via `fnv1a_hash`.
fn partition_for(map: &str, key: &str) -> u32 {
    let combined = format!("{map}:{key}");
    fnv1a_hash(&combined) % PARTITION_COUNT
}

// ---------------------------------------------------------------------------
// WriteBehindDataStore
// ---------------------------------------------------------------------------

/// One entry in the staging overlay: the latest buffered value for a key plus
/// the sequence of the write that produced it.
///
/// The `seq` is the identity check the background flush uses to remove a staging
/// entry only when it still corresponds to the write it just persisted. Without
/// it, a flush that drains an older value (`E1`) and then unconditionally clears
/// the staging slot would wipe a NEWER value (`E2`) that coalesced in after the
/// drain dequeued the batch — dropping read-your-writes back to the now-stale
/// durable value until `E2` itself flushes. That window is what breaks
/// read-your-writes under active eviction, where the resident copy is gone and
/// the staging overlay is the only correct source.
#[derive(Clone)]
struct StagingSlot {
    /// Sequence of the write that set this staged value (`DelayedEntry.sequence`).
    seq: u64,
    /// `Some(value)` = pending write, `None` = pending delete.
    value: Option<RecordValue>,
}

/// Write-behind buffering layer that wraps any [`MapDataStore`].
///
/// Buffers `add`/`remove` calls in per-partition coalesced queues and flushes
/// them to the inner store on a configurable schedule. Reads check the staging
/// area first to provide read-your-writes consistency.
///
/// When a WAL is provided, every write is appended to the WAL and satisfies
/// the configured fsync policy **before** the method returns `Ok(())` so that
/// an acked write is durable even if the process crashes before the background
/// flush loop persists it to the inner store.
pub struct WriteBehindDataStore {
    /// The wrapped persistence store.
    inner: Arc<dyn MapDataStore>,
    /// Configuration controlling flush timing, retries, and capacity.
    config: WriteBehindConfig,
    /// Per-partition write queues, keyed by partition id.
    queues: DashMap<u32, PartitionQueue>,
    /// Staging area for read-your-writes consistency.
    /// `value`: `Some(value)` = pending write, `None` = pending delete.
    staging: DashMap<(String, String), StagingSlot>,
    /// Monotonically increasing operation counter for ordering.
    sequence: AtomicU64,
    /// Node-wide count of pending entries across all partition queues.
    pending_count: AtomicU64,
    /// Signal to wake the flush loop for immediate processing.
    flush_notify: Arc<Notify>,
    /// Shutdown signal sender — wakes the background flush task and tells it to exit.
    shutdown: watch::Sender<bool>,
    /// Set to true once graceful drain begins so any straggler writes are
    /// rejected loudly rather than silently lost.
    is_shutdown: AtomicBool,
    /// Optional WAL for append-before-ack durability. When `Some`, every write
    /// is persisted to the WAL per the configured fsync policy before the
    /// store method returns, closing the unclean-crash loss window.
    wal: Option<Arc<dyn Wal>>,
    /// WAL sequence counter, monotonically increasing per-entry for ordering.
    wal_sequence: AtomicU64,
    /// Entry sequences ASSIGNED but not yet RESOLVED — a write is resolved when
    /// its bytes reach the inner store (flushed) OR a later write to the same key
    /// coalesces it away (retired, its data carried forward by the survivor).
    ///
    /// The smallest still-pending sequence is the PREFIX-COMPLETE flushed
    /// watermark (see [`flushed_watermark`](MapDataStore::flushed_watermark)):
    /// every sequence below it is resolved, with no mid-range hole. This is the
    /// real byte-durability signal the tombstone frontier fences the prune on —
    /// NOT the last-assigned counter, which advances at enqueue and would let a
    /// prune drop a tombstone still buffered in RAM.
    pending_seqs: Mutex<BTreeSet<u64>>,
}

/// WAL plus the live sequence counter's starting value, threaded into
/// `WriteBehindDataStore::new_with_wal`.
///
/// `sequence_start` must be `max_observed_sequence() + 1` computed *after*
/// recovery so the live counter never re-hands a number at or below any
/// persisted log sequence or `.applied` watermark — reusing such a number lets
/// the recovery filter (`e.sequence > applied_seq`) silently drop a post-restart
/// acked write. A fresh node passes `1` (sequence `0` is the reserved
/// "nothing applied" sentinel and is never handed out).
pub struct WalBootstrap {
    /// The WAL implementation every write is appended to before ack.
    pub wal: Arc<dyn Wal>,
    /// First sequence number the live counter hands out (1-based).
    pub sequence_start: u64,
}

impl WriteBehindDataStore {
    /// Creates a new write-behind data store wrapping `inner`.
    ///
    /// Spawns a background flush task that runs until the returned `Arc` (and
    /// all clones) are dropped or shutdown is signalled.
    pub fn new(inner: Arc<dyn MapDataStore>, config: WriteBehindConfig) -> Arc<Self> {
        Self::new_with_wal(inner, config, None)
    }

    /// Creates a new write-behind data store with an optional WAL bootstrap.
    ///
    /// When `bootstrap` is `Some`, every write is appended to the WAL and
    /// satisfies the configured fsync policy before returning `Ok(())`, and the
    /// live sequence counter starts at `bootstrap.sequence_start`. When `None`,
    /// the store behaves identically to `new` (no crash-safe durability
    /// guarantee) and the counter starts at `1` so sequence `0` is never handed
    /// out.
    pub fn new_with_wal(
        inner: Arc<dyn MapDataStore>,
        config: WriteBehindConfig,
        bootstrap: Option<WalBootstrap>,
    ) -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let flush_notify = Arc::new(Notify::new());

        let wal = bootstrap.as_ref().map(|b| b.wal.clone());
        // Seed the live counter from the WAL watermark so post-restart writes
        // never reuse a sequence at or below a persisted watermark (which the
        // recovery filter `e.sequence > applied_seq` would silently drop). A
        // missing bootstrap seeds to 1: sequence 0 is the reserved sentinel.
        let wal_sequence_start = bootstrap.map_or(1, |b| b.sequence_start);

        let store = Arc::new(Self {
            inner,
            config,
            queues: DashMap::new(),
            staging: DashMap::new(),
            sequence: AtomicU64::new(0),
            pending_count: AtomicU64::new(0),
            flush_notify: flush_notify.clone(),
            shutdown: shutdown_tx,
            is_shutdown: AtomicBool::new(false),
            wal,
            wal_sequence: AtomicU64::new(wal_sequence_start),
            pending_seqs: Mutex::new(BTreeSet::new()),
        });

        // Spawn background flush loop with a clone of the Arc
        let store_clone = Arc::clone(&store);
        tokio::spawn(flush_loop(store_clone, shutdown_rx));

        store
    }

    /// Assign the next ordering sequence AND record it pending byte-durability in
    /// one atomic step, under the `pending_seqs` lock. Folding the `fetch_add` and
    /// the set insert under the same lock `flushed_watermark()` reads is what makes
    /// the watermark prefix-complete under concurrency: a separate bump-then-track
    /// leaves a window where the counter is already incremented but the set is
    /// still empty, during which `flushed_watermark()`'s empty-set branch would
    /// return `sequence.load()` — a value ABOVE the just-assigned, still-buffered
    /// sequence (a mid-range hole that could license pruning a non-durable
    /// tombstone). Returns the assigned sequence.
    fn assign_tracked_sequence(&self) -> u64 {
        let mut pending = self.pending_seqs();
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        pending.insert(seq);
        seq
    }

    /// Lock the pending-sequence set, recovering from a poisoned mutex (a prior
    /// panic while holding it leaves a consistent set — a stale entry only holds
    /// the watermark back, the safe direction).
    fn pending_seqs(&self) -> std::sync::MutexGuard<'_, BTreeSet<u64>> {
        self.pending_seqs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record `seq` as enqueued-but-not-yet-durable. Test-only injector for
    /// exercising the watermark deterministically; production assigns and tracks
    /// atomically via [`assign_tracked_sequence`](Self::assign_tracked_sequence).
    #[cfg(test)]
    fn track_pending(&self, seq: u64) {
        self.pending_seqs().insert(seq);
    }

    /// Mark `seq` resolved — its bytes are durable in the inner store (flushed)
    /// or it was superseded by a coalescing write (retired). This is what lets
    /// the prefix-complete flushed watermark advance past `seq`.
    fn resolve_pending(&self, seq: u64) {
        self.pending_seqs().remove(&seq);
    }

    /// Snapshot the pending staging entries for one map as a key-sorted map of
    /// `key -> Option<RecordValue>` (`Some` = buffered write, `None` = pending
    /// delete).
    ///
    /// The staging map is keyed globally by `(map, key)`, so this filters by map
    /// name. The buffer is bounded by `TOPGUN_WRITEBEHIND_CAPACITY`, so the
    /// snapshot is O(buffer), not O(map). Taking it upfront keeps the scan and
    /// leaf overlays single-pass and free of the staging lock during the inner
    /// enumeration.
    fn collect_staging_for_map(&self, map: &str) -> BTreeMap<String, Option<RecordValue>> {
        let mut pending = BTreeMap::new();
        for entry in &self.staging {
            if entry.key().0 == map {
                pending.insert(entry.key().1.clone(), entry.value().value.clone());
            }
        }
        pending
    }

    /// Remove the staging slot for `(map, key)` ONLY if it still corresponds to
    /// the write identified by `flushed_seq`. Returns `true` if it was removed.
    ///
    /// The background flush drains a batch and then persists each entry; the
    /// queue lock is released during the persist, so a newer write can coalesce
    /// into staging (with a higher `seq`) in between. Clearing the slot
    /// unconditionally after a flush would then wipe that newer value, dropping
    /// read-your-writes back to the now-stale durable copy — fatal under active
    /// eviction, where the resident copy is gone and staging is the only correct
    /// source. The `seq` guard removes the slot only when no newer write has
    /// landed, leaving the newer value to be cleared by its own flush.
    fn clear_staging_if_current(&self, map: &str, key: &str, flushed_seq: u64) -> bool {
        self.staging
            .remove_if(&(map.to_string(), key.to_string()), |_, slot| {
                slot.seq == flushed_seq
            })
            .is_some()
    }

    /// Stage the latest buffered `value` for `(map, key)` under sequence `seq`,
    /// keeping monotonicity: a write only replaces the slot if its `seq` is at
    /// least the slot's current `seq`. `seq` is from `assign_tracked_sequence()`,
    /// strictly increasing per call, so a higher `seq` is always the later write.
    /// Two concurrent writers on the same key can otherwise interleave between
    /// the sequence assignment and this insert and let the older write clobber the
    /// newer staged value (a plain `insert` is last-writer-wins by wall-clock,
    /// not by `seq`). The monotonic guard makes the slot's `seq` truthful, which
    /// is the identity [`clear_staging_if_current`] relies on.
    fn stage(&self, map: &str, key: &str, seq: u64, value: Option<RecordValue>) {
        use dashmap::mapref::entry::Entry;
        match self.staging.entry((map.to_string(), key.to_string())) {
            Entry::Occupied(mut e) => {
                if seq >= e.get().seq {
                    e.insert(StagingSlot { seq, value });
                }
            }
            Entry::Vacant(e) => {
                e.insert(StagingSlot { seq, value });
            }
        }
    }

    /// Returns the next WAL sequence number for ordering.
    fn next_wal_sequence(&self) -> u64 {
        self.wal_sequence.fetch_add(1, Ordering::Relaxed)
    }

    /// Appends an entry to the WAL and satisfies the fsync policy before
    /// returning. Returns `Ok(())` immediately when no WAL is configured.
    ///
    /// Must be called before returning `Ok(())` from any mutating method so
    /// that a crash after ack but before the background flush still allows
    /// recovery to replay the acked write.
    async fn wal_append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        if let Some(wal) = &self.wal {
            wal.append(partition, entry).await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Background flush loop (R3)
// ---------------------------------------------------------------------------

/// Background task that periodically flushes eligible entries to the inner store.
///
/// Runs until the shutdown signal is received. Wakes on either the configured
/// interval or an explicit notify (from `soft_flush`/`hard_flush`).
#[allow(clippy::too_many_lines)]
async fn flush_loop(store: Arc<WriteBehindDataStore>, mut shutdown_rx: watch::Receiver<bool>) {
    let interval = tokio::time::Duration::from_millis(store.config.flush_interval_ms);

    loop {
        // Wait for flush interval or early wake, checking shutdown
        tokio::select! {
            () = tokio::time::sleep(interval) => {}
            () = store.flush_notify.notified() => {}
            result = shutdown_rx.changed() => {
                if result.is_err() || *shutdown_rx.borrow() {
                    // Sender dropped or shutdown signalled -- exit loop
                    return;
                }
            }
        }

        // Check shutdown before doing work
        if *shutdown_rx.borrow() {
            return;
        }

        let now = now_millis();
        #[allow(clippy::cast_possible_wrap)]
        let deadline = now.saturating_sub(store.config.write_delay_ms as i64);

        // Collect eligible entries from all partition queues
        let mut ready_entries = Vec::new();
        for mut queue_ref in store.queues.iter_mut() {
            let drained = queue_ref.value_mut().drain_ready(deadline);
            ready_entries.extend(drained);
        }

        if ready_entries.is_empty() {
            continue;
        }

        // Sort by store_time then sequence for fairness
        ready_entries.sort_by(|a, b| {
            a.store_time
                .cmp(&b.store_time)
                .then(a.sequence.cmp(&b.sequence))
        });

        // Process in batches
        for batch in ready_entries.chunks(store.config.batch_size as usize) {
            for entry in batch {
                let result = match &entry.operation {
                    DelayedOp::Store {
                        value,
                        expiration_time,
                    } => {
                        store
                            .inner
                            .add(
                                &entry.map,
                                &entry.key,
                                value,
                                *expiration_time,
                                entry.store_time,
                            )
                            .await
                    }
                    DelayedOp::Remove => {
                        store
                            .inner
                            .remove(&entry.map, &entry.key, entry.store_time)
                            .await
                    }
                };

                let partition_id_for_wal = partition_for(&entry.map, &entry.key);
                match result {
                    Ok(()) => {
                        // Remove this write's staging slot ONLY if it is still the
                        // one we flushed (a newer coalesced write must survive).
                        // pending_count is decremented per terminal flush either
                        // way -- it counts enqueues, one decrement per drained entry.
                        store.clear_staging_if_current(&entry.map, &entry.key, entry.sequence);
                        store.pending_count.fetch_sub(1, Ordering::Relaxed);
                        // Bytes are now durable in the inner store — resolve the
                        // sequence so the prefix-complete flushed watermark can
                        // advance past it. This is the ONLY signal that advances
                        // the tombstone durability fence.
                        store.resolve_pending(entry.sequence);
                        // Mark the WAL entry applied so a clean restart does not
                        // re-replay writes that are already durable in the inner store.
                        if let Some(wal) = &store.wal {
                            if let Err(err) = wal
                                .mark_applied(partition_id_for_wal, entry.wal_sequence)
                                .await
                            {
                                warn!(
                                    map = %entry.map,
                                    key = %entry.key,
                                    wal_seq = entry.wal_sequence,
                                    error = %err,
                                    "Failed to mark WAL entry applied; next restart will re-replay (safe but redundant)"
                                );
                            }
                        }
                    }
                    Err(err) => {
                        let new_retry = entry.retry_count + 1;
                        if new_retry < store.config.max_retries {
                            // Reinsert with incremented retry count
                            let mut retry_entry = entry.clone();
                            retry_entry.retry_count = new_retry;

                            let partition_id = partition_for(&entry.map, &entry.key);
                            let mut queue = store.queues.entry(partition_id).or_default();
                            queue.reinsert_front(vec![retry_entry]);

                            // Backoff before processing next retry-eligible entry
                            let backoff = std::cmp::min(
                                store.config.backoff_base_ms * 2u64.pow(new_retry),
                                store.config.backoff_cap_ms,
                            );
                            tokio::time::sleep(tokio::time::Duration::from_millis(backoff)).await;
                        } else {
                            // Max retries exceeded -- discard and log
                            warn!(
                                map = %entry.map,
                                key = %entry.key,
                                retries = new_retry,
                                error = %err,
                                "Write-behind entry discarded after max retries"
                            );
                            // Same identity guard as the success path: a newer
                            // coalesced write must not be cleared by an older
                            // entry's terminal discard.
                            store.clear_staging_if_current(&entry.map, &entry.key, entry.sequence);
                            store.pending_count.fetch_sub(1, Ordering::Relaxed);
                            // Terminal discard: the sequence will never flush, so
                            // resolve it to unstall the watermark. Its bytes remain
                            // in the WAL until GC (R12(b)), so this does not lose
                            // durability — a discarded write is a re-sync event.
                            store.resolve_pending(entry.sequence);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MapDataStore trait implementation -- Segment S1 (delegating methods)
// ---------------------------------------------------------------------------

#[async_trait]
impl MapDataStore for WriteBehindDataStore {
    async fn add(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        // Reject new writes once graceful drain is in progress so any out-of-order
        // arrival is visible as an error rather than silently lost. In the normal
        // shutdown sequence the HTTP server is already draining before the shutdown
        // flag is set, making this path effectively unreachable under orderly shutdown.
        if self.is_shutdown.load(Ordering::Acquire) {
            anyhow::bail!(
                "write-behind store is shutting down; write rejected for map={map} key={key}"
            );
        }

        // Check capacity before insertion to avoid partial state on rejection
        if self.config.capacity != 0 {
            let current = self.pending_count.load(Ordering::Relaxed);
            // Only reject if this would be a NEW key (not a coalesce).
            // We check the staging area as a fast pre-check; the definitive
            // check happens via PartitionQueue::insert return value.
            let is_new_key = !self
                .staging
                .contains_key(&(map.to_string(), key.to_string()));
            if is_new_key && current >= self.config.capacity {
                anyhow::bail!("Write-behind capacity exceeded");
            }
        }

        let partition_id = partition_for(map, key);
        let wal_seq = self.next_wal_sequence();

        // Append to WAL and satisfy the fsync policy before touching in-memory
        // state. This must happen before returning Ok(()) so a crash after ack
        // still has the write in the WAL for recovery to replay.
        // Persist the full RecordValue so OR-Map adds and tombstones survive a
        // kill -9 in the write-behind window — not just LWW scalars. The WAL
        // entry timestamp is the LWW idempotency-dedup key; OR values carry their
        // own per-entry timestamps inside the record, so it stays None for them.
        let wal_timestamp = if let RecordValue::Lww { timestamp, .. } = value {
            Some(timestamp.clone())
        } else {
            None
        };
        let wal_op = WalOp::Store {
            value: WalStorePayload::Record(value.clone()),
            expiration_time: if expiration_time == 0 {
                None
            } else {
                Some(expiration_time)
            },
        };
        let wal_entry = WalEntry {
            map: map.to_string(),
            key: key.to_string(),
            op: wal_op,
            timestamp: wal_timestamp,
            sequence: wal_seq,
        };
        self.wal_append(partition_id, &wal_entry).await?;

        // Assign and track the durability sequence atomically (see
        // `assign_tracked_sequence`) BEFORE the entry enters the queue.
        let entry_seq = self.assign_tracked_sequence();
        let entry = DelayedEntry {
            map: map.to_string(),
            key: key.to_string(),
            operation: DelayedOp::Store {
                value: value.clone(),
                expiration_time,
            },
            store_time: now,
            sequence: entry_seq,
            retry_count: 0,
            wal_sequence: wal_seq,
        };

        // Insert into partition queue, preserving original store_time on coalesce
        let staging_key = (map.to_string(), key.to_string());
        let retired_seq = {
            let mut queue = self.queues.entry(partition_id).or_default();
            // If coalescing, preserve the original store_time
            if let Some(existing) = queue.value_mut().remove(map, key) {
                let retired = existing.sequence;
                let mut coalesced = entry;
                coalesced.store_time = existing.store_time;
                let _ = queue.value_mut().insert(coalesced);
                // No pending_count change on coalesce
                Some(retired)
            } else {
                let _ = queue.value_mut().insert(entry);
                // New key -- increment pending count
                self.pending_count.fetch_add(1, Ordering::Relaxed);
                None
            }
        };

        // Retire the coalesced-away predecessor (its data is carried forward by
        // this write). Done after releasing the partition-queue lock so the
        // pending-set lock is never nested under it.
        if let Some(retired) = retired_seq {
            self.resolve_pending(retired);
        }

        // Update staging area for read-your-writes
        let (smap, skey) = staging_key;
        self.stage(&smap, &skey, entry_seq, Some(value.clone()));

        Ok(())
    }

    async fn add_backup(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        expiration_time: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        // Backups pass through directly -- backup replication has its own
        // consistency guarantees
        self.inner
            .add_backup(map, key, value, expiration_time, now)
            .await
    }

    async fn remove(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        // Reject once graceful drain begins — same reasoning as `add`.
        if self.is_shutdown.load(Ordering::Acquire) {
            anyhow::bail!(
                "write-behind store is shutting down; remove rejected for map={map} key={key}"
            );
        }

        // Check capacity before insertion
        if self.config.capacity != 0 {
            let current = self.pending_count.load(Ordering::Relaxed);
            let is_new_key = !self
                .staging
                .contains_key(&(map.to_string(), key.to_string()));
            if is_new_key && current >= self.config.capacity {
                anyhow::bail!("Write-behind capacity exceeded");
            }
        }

        let partition_id = partition_for(map, key);
        let wal_seq = self.next_wal_sequence();

        // Append to WAL before updating in-memory state so crash recovery can
        // replay the tombstone if the process dies after ack but before flush.
        let wal_entry = WalEntry {
            map: map.to_string(),
            key: key.to_string(),
            op: WalOp::Remove,
            timestamp: None,
            sequence: wal_seq,
        };
        self.wal_append(partition_id, &wal_entry).await?;

        // Assign and track the durability sequence atomically before queuing.
        let entry_seq = self.assign_tracked_sequence();
        let entry = DelayedEntry {
            map: map.to_string(),
            key: key.to_string(),
            operation: DelayedOp::Remove,
            store_time: now,
            sequence: entry_seq,
            retry_count: 0,
            wal_sequence: wal_seq,
        };

        let staging_key = (map.to_string(), key.to_string());
        let retired_seq = {
            let mut queue = self.queues.entry(partition_id).or_default();
            if let Some(existing) = queue.value_mut().remove(map, key) {
                let retired = existing.sequence;
                let mut coalesced = entry;
                coalesced.store_time = existing.store_time;
                let _ = queue.value_mut().insert(coalesced);
                // No pending_count change on coalesce
                Some(retired)
            } else {
                let _ = queue.value_mut().insert(entry);
                self.pending_count.fetch_add(1, Ordering::Relaxed);
                None
            }
        };

        if let Some(retired) = retired_seq {
            self.resolve_pending(retired);
        }

        // Pending delete marker in staging
        let (smap, skey) = staging_key;
        self.stage(&smap, &skey, entry_seq, None);

        Ok(())
    }

    async fn remove_backup(&self, map: &str, key: &str, now: i64) -> anyhow::Result<()> {
        // Backups pass through directly
        self.inner.remove_backup(map, key, now).await
    }

    async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
        let staging_key = (map.to_string(), key.to_string());

        // Check staging first for read-your-writes consistency
        if let Some(entry) = self.staging.get(&staging_key) {
            return match &entry.value().value {
                Some(value) => Ok(Some(value.clone())),
                // Pending delete -- do not consult inner store
                None => Ok(None),
            };
        }

        // Not in staging -- delegate to inner store
        self.inner.load(map, key).await
    }

    async fn load_all(
        &self,
        map: &str,
        keys: &[String],
    ) -> anyhow::Result<Vec<(String, RecordValue)>> {
        let mut results = Vec::new();
        let mut inner_keys = Vec::new();

        for key in keys {
            let staging_key = (map.to_string(), key.clone());
            if let Some(entry) = self.staging.get(&staging_key) {
                if let Some(value) = &entry.value().value {
                    results.push((key.clone(), value.clone()));
                }
                // Pending delete (None) -- skip entirely, do not fetch from inner
            } else {
                inner_keys.push(key.clone());
            }
        }

        // Batch-load remaining keys from the inner store
        if !inner_keys.is_empty() {
            let inner_results = self.inner.load_all(map, &inner_keys).await?;
            results.extend(inner_results);
        }

        Ok(results)
    }

    async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
        // Reject new writes once graceful drain is in progress so any out-of-order
        // arrival is visible as an error rather than silently lost, matching the
        // add/remove guards. Effectively unreachable under orderly shutdown (the
        // HTTP server drains before the shutdown flag is set).
        if self.is_shutdown.load(Ordering::Acquire) {
            anyhow::bail!("write-behind store is shutting down; remove_all rejected for map={map}");
        }

        let now = now_millis();

        for key in keys {
            // Check capacity before each insertion
            if self.config.capacity != 0 {
                let current = self.pending_count.load(Ordering::Relaxed);
                let is_new_key = !self.staging.contains_key(&(map.to_string(), key.clone()));
                if is_new_key && current >= self.config.capacity {
                    anyhow::bail!("Write-behind capacity exceeded");
                }
            }

            let partition_id = partition_for(map, key);
            let wal_seq = self.next_wal_sequence();

            // Append each tombstone to the WAL before queuing so crash recovery
            // can replay every individual key deletion, not just the batch call.
            let wal_entry = WalEntry {
                map: map.to_string(),
                key: key.clone(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: wal_seq,
            };
            self.wal_append(partition_id, &wal_entry).await?;

            // Assign and track the durability sequence atomically before queuing.
            let entry_seq = self.assign_tracked_sequence();
            let entry = DelayedEntry {
                map: map.to_string(),
                key: key.clone(),
                operation: DelayedOp::Remove,
                store_time: now,
                sequence: entry_seq,
                retry_count: 0,
                wal_sequence: wal_seq,
            };

            let staging_key = (map.to_string(), key.clone());
            let retired_seq = {
                let mut queue = self.queues.entry(partition_id).or_default();
                if let Some(existing) = queue.value_mut().remove(map, key) {
                    let retired = existing.sequence;
                    let mut coalesced = entry;
                    coalesced.store_time = existing.store_time;
                    let _ = queue.value_mut().insert(coalesced);
                    Some(retired)
                } else {
                    let _ = queue.value_mut().insert(entry);
                    self.pending_count.fetch_add(1, Ordering::Relaxed);
                    None
                }
            };

            // Retire the coalesced-away predecessor so its durability sequence does
            // not stall the flushed watermark forever (a leaked pending sequence
            // would permanently pin the prune). Done after the queue lock is
            // released so the pending-set lock is never nested under it.
            if let Some(retired) = retired_seq {
                self.resolve_pending(retired);
            }

            let (smap, skey) = staging_key;
            self.stage(&smap, &skey, entry_seq, None);
        }

        Ok(())
    }

    async fn list_maps(&self) -> anyhow::Result<Vec<String>> {
        // Start from the durable catalog so a map persisted before a restart is
        // discovered even when not resident.
        let mut names = self.inner.list_maps().await?;

        // Union in maps whose only write is still buffered in staging. Before
        // SPEC-323 clean-marking, a buffered record was necessarily also
        // resident, so the durable catalog covered it. R6 makes a buffered
        // record evictable, so a map whose sole write is buffered-and-evicted is
        // now reachable only via staging — it must be surfaced here or the
        // residency-independent seed would miss it. A map present in staging
        // only via pending deletes (None) contributes nothing.
        let mut staging_only: Vec<String> = self
            .staging
            .iter()
            .filter(|entry| entry.value().value.is_some())
            .map(|entry| entry.key().0.clone())
            .collect();
        staging_only.sort();
        staging_only.dedup();
        for name in staging_only {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    async fn enumerate_leaves(
        &self,
        map: &str,
        is_backup: bool,
        sink: &mut dyn LeafSink,
    ) -> anyhow::Result<()> {
        // Backup scans carry no staging overlay: add_backup/remove_backup go
        // straight to inner, so staging holds only non-backup writes. Overlaying
        // would double-count or falsely surface a primary write onto a backup
        // tree.
        if is_backup {
            return self.inner.enumerate_leaves(map, is_backup, sink).await;
        }

        // Collect the map's pending staging set upfront so the durable
        // enumeration can be overlaid in a single pass. The buffer is bounded by
        // TOPGUN_WRITEBEHIND_CAPACITY, so this stays O(buffer), not O(map).
        let pending = self.collect_staging_for_map(map);

        // Overlay the durable leaves with the buffered state: a buffered Some
        // replaces the durable leaf hash (recomputed from the buffered value),
        // and a buffered None (pending delete) suppresses the durable leaf so an
        // evicted-but-deleted key cannot resurrect. The sink is invoked through
        // an overlay adapter so peak memory stays bounded exactly as the durable
        // path bounds it.
        let mut overlay = StagingLeafSink::new(&pending, sink);
        self.inner
            .enumerate_leaves(map, is_backup, &mut overlay)
            .await?;

        // Emit staging-only leaves (buffered Some keys the durable store never
        // had) after the durable enumeration completes. merkle_leaf_hash returns
        // None for OrMap entries that must contribute no leaf; propagate that
        // None so a rebuilt root stays byte-identical to the live root.
        let mut extra: Vec<MerkleLeaf> = Vec::new();
        for (key, value) in &pending {
            if let Some(value) = value {
                if !overlay.was_seen(key) {
                    if let Some((kind, leaf_hash)) = merkle_leaf_hash(key, value) {
                        extra.push(MerkleLeaf {
                            key: key.clone(),
                            kind,
                            leaf_hash,
                        });
                    }
                }
            }
        }
        if !extra.is_empty() {
            sink.consume(extra).await?;
        }
        Ok(())
    }

    async fn scan_values(
        &self,
        map: &str,
        is_backup: bool,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        // Backup scans never carry staging overlay (see enumerate_leaves).
        if is_backup {
            return self.inner.scan_values(map, is_backup, max_batch_cost).await;
        }

        let pending = self.collect_staging_for_map(map);
        let batch = self
            .inner
            .scan_values(map, is_backup, max_batch_cost)
            .await?;
        // The first batch is the only place staging-only keys are emitted, so
        // they are surfaced exactly once across the whole resumable scan.
        Ok(overlay_scan_batch(batch, &pending, true))
    }

    async fn scan_values_batched(
        &self,
        map: &str,
        is_backup: bool,
        cursor: ScanCursor,
        max_batch_cost: u64,
    ) -> anyhow::Result<ScanBatch> {
        // Backup scans never carry staging overlay (see enumerate_leaves).
        if is_backup {
            return self
                .inner
                .scan_values_batched(map, is_backup, cursor, max_batch_cost)
                .await;
        }

        let pending = self.collect_staging_for_map(map);
        let batch = self
            .inner
            .scan_values_batched(map, is_backup, cursor, max_batch_cost)
            .await?;
        // Resumed batches only overlay (replace/suppress) durable rows; the
        // staging-only keys were already emitted by the first scan_values call,
        // so emitting them again here would double-count.
        Ok(overlay_scan_batch(batch, &pending, false))
    }

    fn is_loadable(&self, _key: &str) -> bool {
        // Staging area handles consistency, so always loadable
        true
    }

    fn pending_operation_count(&self) -> u64 {
        self.pending_count.load(Ordering::Relaxed)
    }

    async fn soft_flush(&self) -> anyhow::Result<u64> {
        // Notify the background task to flush immediately
        self.flush_notify.notify_one();
        Ok(self.sequence.load(Ordering::Relaxed))
    }

    fn assigned_write_sequence(&self) -> u64 {
        // One past the highest sequence handed out — an upper bound on any
        // in-flight write's sequence. The tombstone frontier snapshots this at
        // stamp time; because the tombstone's own byte-write was enqueued (and
        // thus assigned a sequence) strictly before the stamp, this is `>=` it.
        self.sequence.load(Ordering::Relaxed)
    }

    fn flushed_watermark(&self) -> u64 {
        // Prefix-complete: the smallest still-pending sequence is the first
        // un-resolved write; every sequence below it is durable (flushed) or
        // retired (coalesced away). With nothing pending, all assigned writes
        // are resolved, so the watermark is the full assigned counter. This can
        // NEVER expose a value above an un-flushed sequence — the property that
        // makes `stamped_seq <= flushed_watermark()` a sound durability fence.
        let pending = self.pending_seqs();
        match pending.iter().next().copied() {
            Some(min_pending) => min_pending,
            None => self.sequence.load(Ordering::Relaxed),
        }
    }

    async fn hard_flush(&self) -> anyhow::Result<()> {
        // Mark the store as shutting down so any straggler writes are rejected
        // rather than queued behind the drain.
        self.is_shutdown.store(true, Ordering::Release);

        // Stop the background flush loop so it doesn't race with our direct drain.
        let _ = self.shutdown.send(true);

        let timeout_duration = tokio::time::Duration::from_millis(self.config.shutdown_timeout_ms);
        let deadline = tokio::time::Instant::now() + timeout_duration;

        // Flush all partition queues directly to the inner store. Using i64::MAX
        // as the drain deadline ensures every buffered entry is eligible regardless
        // of its write_delay_ms schedule — on shutdown we want everything, not just
        // entries past their delay window.
        loop {
            // Collect all entries still in the queues.
            let mut all_entries: Vec<DelayedEntry> = Vec::new();
            for mut queue_ref in self.queues.iter_mut() {
                let drained = queue_ref.value_mut().drain_ready(i64::MAX);
                all_entries.extend(drained);
            }

            if all_entries.is_empty() {
                return Ok(());
            }

            // Persist each entry, wrapping each inner-store call with the
            // remaining deadline budget. This ensures that even a single slow
            // inner-store write cannot hold up the process past the configured
            // timeout.
            let mut timed_out_entries: Vec<DelayedEntry> = Vec::new();

            for entry in all_entries {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    // Budget exhausted — collect this entry as timed-out.
                    timed_out_entries.push(entry);
                    continue;
                }

                let flush_future = async {
                    match &entry.operation {
                        DelayedOp::Store {
                            value,
                            expiration_time,
                        } => {
                            self.inner
                                .add(
                                    &entry.map,
                                    &entry.key,
                                    value,
                                    *expiration_time,
                                    entry.store_time,
                                )
                                .await
                        }
                        DelayedOp::Remove => {
                            self.inner
                                .remove(&entry.map, &entry.key, entry.store_time)
                                .await
                        }
                    }
                };

                let partition_id_for_wal = partition_for(&entry.map, &entry.key);
                match tokio::time::timeout(remaining, flush_future).await {
                    Ok(Ok(())) => {
                        self.staging.remove(&(entry.map.clone(), entry.key.clone()));
                        self.pending_count.fetch_sub(1, Ordering::Relaxed);
                        self.resolve_pending(entry.sequence);
                        // Mark WAL applied so a restart after clean shutdown is
                        // a no-op rather than re-replaying already-durable writes.
                        if let Some(wal) = &self.wal {
                            if let Err(err) = wal
                                .mark_applied(partition_id_for_wal, entry.wal_sequence)
                                .await
                            {
                                warn!(
                                    map = %entry.map,
                                    key = %entry.key,
                                    wal_seq = entry.wal_sequence,
                                    error = %err,
                                    "Failed to mark WAL entry applied during shutdown drain"
                                );
                            }
                        }
                    }
                    Ok(Err(err)) => {
                        // Inner store returned an error; log and treat as timed-out
                        // rather than looping forever on a failing backend.
                        warn!(
                            map = %entry.map,
                            key = %entry.key,
                            error = %err,
                            "Inner store error during shutdown drain; entry not persisted"
                        );
                        timed_out_entries.push(entry);
                    }
                    Err(_elapsed) => {
                        // Deadline expired mid-write.
                        timed_out_entries.push(entry);
                    }
                }
            }

            if !timed_out_entries.is_empty() {
                // Also sweep any entries reinserted by concurrent retry paths.
                for mut queue_ref in self.queues.iter_mut() {
                    let drained = queue_ref.value_mut().drain_ready(i64::MAX);
                    timed_out_entries.extend(drained);
                }

                // Log every op that could not be persisted within the timeout.
                for entry in &timed_out_entries {
                    warn!(
                        map = %entry.map,
                        key = %entry.key,
                        timeout_ms = self.config.shutdown_timeout_ms,
                        "Shutdown drain timed out; write-behind entry could not be persisted"
                    );
                }
                return Ok(());
            }

            // Yield briefly between iterations to avoid starving the tokio runtime.
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    async fn flush_key(
        &self,
        map: &str,
        key: &str,
        value: &RecordValue,
        is_backup: bool,
    ) -> anyhow::Result<()> {
        if is_backup {
            // Backup flush passes through directly
            return self.inner.flush_key(map, key, value, is_backup).await;
        }

        // Remove the key from the partition queue if present
        let partition_id = partition_for(map, key);
        let removed = if let Some(mut queue) = self.queues.get_mut(&partition_id) {
            queue.remove(map, key)
        } else {
            None
        };

        // Remove from staging
        self.staging.remove(&(map.to_string(), key.to_string()));

        // Decrement pending count and resolve the buffered entry's durability
        // sequence if the key was actually in the queue — this direct flush
        // supersedes it, so its sequence must not stall the flushed watermark.
        if let Some(removed) = &removed {
            self.pending_count.fetch_sub(1, Ordering::Relaxed);
            self.resolve_pending(removed.sequence);
        }

        // Persist the caller-provided value directly to the inner store
        let now = now_millis();
        self.inner.add(map, key, value, 0, now).await
    }

    fn reset(&self) {
        self.queues.clear();
        self.staging.clear();
        self.sequence.store(0, Ordering::Relaxed);
        self.pending_count.store(0, Ordering::Relaxed);
        self.pending_seqs().clear();
        self.inner.reset();
    }

    fn is_null(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Staging overlay helpers for the buffer-aware read surface
// ---------------------------------------------------------------------------

/// Overlay a map's pending staging set onto one durable [`ScanBatch`].
///
/// For each durable row: a buffered `Some(v)` replaces the row with the newer
/// buffered value; a buffered `None` (pending delete) suppresses the row so an
/// evicted-but-deleted key cannot resurrect; an absent staging entry keeps the
/// durable row unchanged. When `emit_staging_only` is true (the first batch of a
/// resumable scan), staging-only `Some` keys the durable store never returned
/// are appended once. Resumed batches pass `false` so those keys are never
/// double-counted.
fn overlay_scan_batch(
    batch: ScanBatch,
    pending: &BTreeMap<String, Option<RecordValue>>,
    emit_staging_only: bool,
) -> ScanBatch {
    if pending.is_empty() {
        return batch;
    }

    let ScanBatch {
        records,
        next_cursor,
    } = batch;

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<(String, RecordValue)> = Vec::with_capacity(records.len());
    for (key, value) in records {
        match pending.get(&key) {
            Some(Some(buffered)) => {
                // Buffered write is newer than the durable row; emit it instead.
                seen.insert(key.clone());
                out.push((key, buffered.clone()));
            }
            Some(None) => {
                // Pending delete hides the durable value.
            }
            None => {
                out.push((key, value));
            }
        }
    }

    if emit_staging_only {
        for (key, value) in pending {
            if let Some(value) = value {
                if !seen.contains(key) {
                    out.push((key.clone(), value.clone()));
                }
            }
        }
    }

    ScanBatch {
        records: out,
        next_cursor,
    }
}

/// A [`LeafSink`] adapter that overlays a map's pending staging set onto the
/// durable leaf stream before forwarding to the real sink.
///
/// A buffered `Some` recomputes the leaf hash from the buffered value (so a
/// buffered-then-flushed record yields an identical leaf, leaving the Merkle
/// root unchanged); a buffered `None` suppresses the durable leaf. Keys it has
/// emitted are tracked so the outer enumeration can append the staging-only
/// leaves exactly once.
struct StagingLeafSink<'a> {
    pending: &'a BTreeMap<String, Option<RecordValue>>,
    inner: &'a mut dyn LeafSink,
    seen: std::collections::HashSet<String>,
}

impl<'a> StagingLeafSink<'a> {
    fn new(
        pending: &'a BTreeMap<String, Option<RecordValue>>,
        inner: &'a mut dyn LeafSink,
    ) -> Self {
        Self {
            pending,
            inner,
            seen: std::collections::HashSet::new(),
        }
    }

    fn was_seen(&self, key: &str) -> bool {
        self.seen.contains(key)
    }
}

#[async_trait]
impl LeafSink for StagingLeafSink<'_> {
    async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
        let mut out: Vec<MerkleLeaf> = Vec::with_capacity(batch.len());
        for leaf in batch {
            match self.pending.get(&leaf.key) {
                Some(Some(buffered)) => {
                    self.seen.insert(leaf.key.clone());
                    // merkle_leaf_hash returns None for OrMap entries that must
                    // contribute no leaf; propagate that None (emit nothing)
                    // rather than injecting a zero/placeholder leaf.
                    if let Some((kind, leaf_hash)) = merkle_leaf_hash(&leaf.key, buffered) {
                        out.push(MerkleLeaf {
                            key: leaf.key,
                            kind,
                            leaf_hash,
                        });
                    }
                }
                Some(None) => {
                    // Pending delete suppresses the durable leaf.
                }
                None => {
                    out.push(leaf);
                }
            }
        }
        if out.is_empty() {
            return Ok(());
        }
        self.inner.consume(out).await
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn dummy_value() -> RecordValue {
        RecordValue::Lww {
            value: Value::Null,
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    fn dummy_value_with(millis: u64) -> RecordValue {
        RecordValue::Lww {
            value: Value::Null,
            timestamp: Timestamp {
                millis,
                counter: 0,
                node_id: String::new(),
            },
        }
    }

    fn short_delay_config() -> WriteBehindConfig {
        WriteBehindConfig {
            write_delay_ms: 10,
            flush_interval_ms: 10,
            batch_size: 100,
            max_retries: 3,
            backoff_base_ms: 10,
            backoff_cap_ms: 100,
            capacity: 0,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        }
    }

    // -----------------------------------------------------------------------
    // SpyDataStore
    // -----------------------------------------------------------------------

    /// Records which calls were made to the inner store, for test assertions.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    enum SpyCall {
        Add { map: String, key: String },
        Remove { map: String, key: String },
    }

    /// Test-only data store that records calls and optionally returns pre-seeded values.
    struct SpyDataStore {
        calls: Arc<Mutex<Vec<SpyCall>>>,
        /// Pre-seeded values returned by `load()`.
        seeded: DashMap<(String, String), RecordValue>,
    }

    impl SpyDataStore {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                seeded: DashMap::new(),
            }
        }

        fn calls(&self) -> Arc<Mutex<Vec<SpyCall>>> {
            Arc::clone(&self.calls)
        }

        /// Pre-seed a value so `load()` returns it (simulates persisted data).
        fn seed(&self, map: &str, key: &str, value: RecordValue) {
            self.seeded
                .insert((map.to_string(), key.to_string()), value);
        }
    }

    #[async_trait]
    impl MapDataStore for SpyDataStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(SpyCall::Add {
                map: map.to_string(),
                key: key.to_string(),
            });
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(SpyCall::Remove {
                map: map.to_string(),
                key: key.to_string(),
            });
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, map: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(self
                .seeded
                .get(&(map.to_string(), key.to_string()))
                .map(|v| v.value().clone()))
        }

        async fn load_all(
            &self,
            map: &str,
            keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            let mut results = Vec::new();
            for key in keys {
                if let Some(v) = self.seeded.get(&(map.to_string(), key.clone())) {
                    results.push((key.clone(), v.value().clone()));
                }
            }
            Ok(results)
        }

        async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            // Test spy holds no durable leaves.
            Ok(())
        }

        async fn scan_values(
            &self,
            _map: &str,
            _is_backup: bool,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        async fn scan_values_batched(
            &self,
            _map: &str,
            _is_backup: bool,
            _cursor: ScanCursor,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        fn is_loadable(&self, _key: &str) -> bool {
            true
        }

        fn pending_operation_count(&self) -> u64 {
            0
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn flush_key(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _is_backup: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn reset(&self) {}

        fn is_null(&self) -> bool {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: enqueue_and_load_returns_staged_value
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn enqueue_and_load_returns_staged_value() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            // Long delay so the flush loop never fires during the test
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();

        // load() should return the staged value immediately
        let loaded = store.load("map1", "key1").await.unwrap();
        assert!(
            loaded.is_some(),
            "Staged value should be returned by load()"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: coalescing_preserves_store_time
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn coalescing_preserves_store_time() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val1 = dummy_value_with(1);
        let val2 = dummy_value_with(2);

        // First write at store_time=1000
        store.add("map1", "key1", &val1, 0, 1000).await.unwrap();
        // Second write at store_time=2000 (coalesces)
        store.add("map1", "key1", &val2, 0, 2000).await.unwrap();

        // Only one pending entry (coalesced)
        assert_eq!(
            store.pending_operation_count(),
            1,
            "Coalesced writes should count as one pending entry"
        );

        // Verify the queue has the original store_time but latest value
        let partition_id = partition_for("map1", "key1");
        let queue = store.queues.get(&partition_id).unwrap();
        let entry = queue
            .entries
            .get(&("map1".to_string(), "key1".to_string()))
            .unwrap();
        assert_eq!(
            entry.store_time, 1000,
            "Coalesced entry should preserve original store_time"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: flush_persists_to_inner
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn flush_persists_to_inner() {
        let spy = Arc::new(SpyDataStore::new());
        let calls = spy.calls();
        let inner: Arc<dyn MapDataStore> = spy;
        let config = short_delay_config();
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();

        // Wait for the flush loop to process the entry
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let recorded = calls.lock().unwrap();
        assert!(
            recorded
                .iter()
                .any(|c| matches!(c, SpyCall::Add { map, key } if map == "map1" && key == "key1")),
            "Inner store should have received the add call after flush"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: pending_count_tracks_operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn pending_count_tracks_operations() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "a", &val, 0, 1000).await.unwrap();
        store.add("map1", "b", &val, 0, 1000).await.unwrap();
        store.add("map1", "c", &val, 0, 1000).await.unwrap();

        assert_eq!(store.pending_operation_count(), 3);

        // Flush all entries
        store.hard_flush().await.unwrap();

        assert_eq!(store.pending_operation_count(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 5: capacity_limit_rejects_excess
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn capacity_limit_rejects_excess() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            capacity: 2,
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "a", &val, 0, 1000).await.unwrap();
        store.add("map1", "b", &val, 0, 1000).await.unwrap();

        // Third key should be rejected
        let result = store.add("map1", "c", &val, 0, 1000).await;
        assert!(result.is_err(), "Should reject writes exceeding capacity");

        // Queue still contains exactly 2 entries
        assert_eq!(store.pending_operation_count(), 2);
    }

    // -----------------------------------------------------------------------
    // Test 6: hard_flush_drains_all
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn hard_flush_drains_all() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        for i in 0..5 {
            store
                .add("map1", &format!("key{i}"), &val, 0, 1000)
                .await
                .unwrap();
        }

        store.hard_flush().await.unwrap();
        assert_eq!(store.pending_operation_count(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 7: remove_enqueues_delete_op
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn remove_enqueues_delete_op() {
        let spy = Arc::new(SpyDataStore::new());
        // Pre-seed the inner store so it would return a value if consulted
        spy.seed("map1", "key1", dummy_value());
        let inner: Arc<dyn MapDataStore> = spy;

        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();
        store.remove("map1", "key1", 2000).await.unwrap();

        // load() should return None because staging has a pending delete marker,
        // NOT falling through to the inner store (which has a pre-seeded value)
        let loaded = store.load("map1", "key1").await.unwrap();
        assert!(
            loaded.is_none(),
            "Pending delete should cause load() to return None without consulting inner store"
        );
    }

    // -----------------------------------------------------------------------
    // Test 8: flush_key_persists_immediately
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn flush_key_persists_immediately() {
        let spy = Arc::new(SpyDataStore::new());
        let calls = spy.calls();
        let inner: Arc<dyn MapDataStore> = spy;

        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "key1", &val, 0, 1000).await.unwrap();
        assert_eq!(store.pending_operation_count(), 1);

        // flush_key with is_backup=false should persist immediately
        let flush_val = dummy_value_with(42);
        store
            .flush_key("map1", "key1", &flush_val, false)
            .await
            .unwrap();

        // Staging should be cleared
        assert!(
            store
                .staging
                .get(&("map1".to_string(), "key1".to_string()))
                .is_none(),
            "Staging should be cleared after flush_key"
        );

        // Pending count should be decremented
        assert_eq!(store.pending_operation_count(), 0);

        // Inner store should have received an add call
        let recorded = calls.lock().unwrap();
        assert!(
            recorded
                .iter()
                .any(|c| matches!(c, SpyCall::Add { map, key } if map == "map1" && key == "key1")),
            "Inner store should have received the flush_key add call"
        );
    }

    // -----------------------------------------------------------------------
    // AC3c: the flushed watermark is PREFIX-COMPLETE — it never exposes a value
    // above a still-pending (un-flushed) sequence, even when a HIGHER sequence
    // is resolved out of order below it. A max-with-holes watermark would admit
    // a mid-range-hole kill -9 tombstone resurrection; this proves no hole is
    // ever exposed.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn ac3c_flushed_watermark_prefix_complete_never_exposes_hole() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        // Long delays so the background flush loop never touches our injected
        // pending set — this test drives resolution deterministically.
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        // Three writes enqueued: sequences 1, 2, 3 pending, nothing flushed yet.
        store.track_pending(1);
        store.track_pending(2);
        store.track_pending(3);
        assert_eq!(
            store.flushed_watermark(),
            1,
            "smallest pending sequence is the watermark — everything below it is resolved"
        );

        // Resolve the MIDDLE sequence (2) out of order: 1 is still pending, so
        // the watermark must NOT jump past the hole at 1 to 2 or 3.
        store.resolve_pending(2);
        assert_eq!(
            store.flushed_watermark(),
            1,
            "out-of-order resolve of 2 must not expose a value above the still-pending 1"
        );

        // Resolve the lowest (1): now 1 and 2 are resolved, only 3 pends, so the
        // watermark advances to exactly 3 — never skipping over an un-flushed seq.
        store.resolve_pending(1);
        assert_eq!(
            store.flushed_watermark(),
            3,
            "watermark advances to the next pending sequence, 3 — prefix stays complete"
        );

        // Resolve the last: nothing pending, watermark is the full assigned counter.
        store.resolve_pending(3);
        assert_eq!(
            store.flushed_watermark(),
            store.assigned_write_sequence(),
            "with nothing pending the watermark is the assigned-sequence counter"
        );
    }

    // -----------------------------------------------------------------------
    // AC3c (real path): a genuine add → in-order background/hard flush advances
    // the watermark monotonically, and a still-buffered write holds it back.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn ac3c_real_flush_advances_watermark_only_on_byte_durability() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("m", "k1", &val, 0, 1).await.unwrap();
        store.add("m", "k2", &val, 0, 1).await.unwrap();

        // Both writes are buffered (write_delay is 60s), so the watermark sits at
        // the lowest pending sequence — no byte durability yet.
        let assigned = store.assigned_write_sequence();
        assert!(
            store.flushed_watermark() < assigned,
            "buffered writes are not byte-durable: watermark {} must be below assigned {}",
            store.flushed_watermark(),
            assigned
        );

        // Drain everything to the inner store (deterministic byte durability).
        store.hard_flush().await.unwrap();
        assert_eq!(
            store.flushed_watermark(),
            assigned,
            "after a full drain every assigned sequence is durable, so the watermark reaches it"
        );
    }

    // -----------------------------------------------------------------------
    // A batch `remove_all` that coalesces a still-buffered write MUST resolve the
    // retired sequence — otherwise it leaks in the pending set forever and pins
    // the flushed watermark below it, permanently stalling the tombstone prune.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn remove_all_coalesce_resolves_retired_sequence_no_watermark_stall() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        // A buffered add for k1: its sequence is the lowest pending, so the
        // watermark sits at it (nothing below it is durable yet).
        store.add("m", "k1", &dummy_value(), 0, 1).await.unwrap();
        let add_seq = store.flushed_watermark();

        // A batch remove of the SAME key coalesces the buffered add. The retired
        // add sequence must be resolved so the watermark can advance past it —
        // before the fix it leaked and the watermark stayed pinned at `add_seq`.
        store.remove_all("m", &["k1".to_string()]).await.unwrap();
        assert!(
            store.flushed_watermark() > add_seq,
            "coalesced-away add sequence {add_seq} must be resolved, not leaked: watermark is {}",
            store.flushed_watermark()
        );

        // And after a full drain the watermark reaches the assigned counter —
        // proving no pending sequence was orphaned by the coalesce.
        store.hard_flush().await.unwrap();
        assert_eq!(
            store.flushed_watermark(),
            store.assigned_write_sequence(),
            "no orphaned pending sequence after coalesce + drain"
        );
    }

    // -----------------------------------------------------------------------
    // Test 9: soft_flush_returns_sequence
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn soft_flush_returns_sequence() {
        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        store.add("map1", "a", &val, 0, 1000).await.unwrap();
        store.add("map1", "b", &val, 0, 1000).await.unwrap();

        let seq = store.soft_flush().await.unwrap();
        assert!(seq > 0, "Sequence should be > 0 after enqueuing entries");
    }

    // -----------------------------------------------------------------------
    // Test 10: hard_flush_drains_all_to_inner_store (AC1)
    //
    // Verifies that every write acked to a client is readable from the inner
    // store after hard_flush() completes. This covers the graceful-shutdown
    // guarantee: no acked write is lost when the process exits cleanly.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn hard_flush_drains_all_to_inner_store() {
        // Use a SpyDataStore that also persists values so load() works post-drain.
        let spy = Arc::new(SpyDataStore::new());
        let inner: Arc<dyn MapDataStore> = Arc::clone(&spy) as Arc<dyn MapDataStore>;

        let config = WriteBehindConfig {
            // Long write delay ensures the background loop would NOT flush on its own
            // during this test — only hard_flush() should drive persistence.
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(inner, config);

        let val = dummy_value();
        let keys = ["alpha", "beta", "gamma", "delta"];
        for key in &keys {
            store.add("drain_map", key, &val, 0, 1000).await.unwrap();
        }

        // All writes are acked (add returned Ok) but not yet in the inner store.
        assert_eq!(store.pending_operation_count(), keys.len() as u64);

        // hard_flush() should drain every pending entry.
        store.hard_flush().await.unwrap();

        assert_eq!(
            store.pending_operation_count(),
            0,
            "All pending entries should be drained after hard_flush"
        );

        // Verify the inner store received an add call for every key.
        // Drop the lock guard before any await point to satisfy clippy's
        // `await_holding_lock` lint (holding a MutexGuard across an await
        // can deadlock if another task also tries to acquire the same mutex).
        {
            let recorded = spy.calls.lock().unwrap();
            for key in &keys {
                assert!(
                    recorded.iter().any(
                        |c| matches!(c, SpyCall::Add { map, key: k } if map == "drain_map" && k == *key)
                    ),
                    "Inner store should have received add for key={key}"
                );
            }
        } // recorded guard dropped here

        // Writes after shutdown must be rejected.
        let result = store.add("drain_map", "late_write", &val, 0, 1000).await;
        assert!(
            result.is_err(),
            "Writes after hard_flush (shutdown) must be rejected"
        );
    }

    // -----------------------------------------------------------------------
    // Test 11: hard_flush_timeout_logs_warn_and_returns (AC2)
    //
    // Injects a slow inner store to verify that hard_flush() respects
    // shutdown_timeout_ms: on timeout it logs each pending op via warn! and
    // returns Ok(()) rather than hanging the process.
    // -----------------------------------------------------------------------

    /// Inner store that sleeps for a configurable duration on every `add` call,
    /// allowing tests to simulate a slow or unresponsive persistence backend.
    struct SlowDataStore {
        delay_ms: u64,
        calls: Arc<Mutex<Vec<SpyCall>>>,
    }

    impl SlowDataStore {
        fn new(delay_ms: u64) -> Self {
            Self {
                delay_ms,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl MapDataStore for SlowDataStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            // Simulate a slow inner store that takes longer than the drain timeout.
            tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
            self.calls.lock().unwrap().push(SpyCall::Add {
                map: map.to_string(),
                key: key.to_string(),
            });
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _expiration_time: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
            Ok(())
        }

        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(&self, _map: &str, _key: &str) -> anyhow::Result<Option<RecordValue>> {
            Ok(None)
        }

        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            Ok(Vec::new())
        }

        async fn remove_all(&self, _map: &str, _keys: &[String]) -> anyhow::Result<()> {
            Ok(())
        }

        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn LeafSink,
        ) -> anyhow::Result<()> {
            // Slow test store has no durable leaves to enumerate.
            Ok(())
        }

        async fn scan_values(
            &self,
            _map: &str,
            _is_backup: bool,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        async fn scan_values_batched(
            &self,
            _map: &str,
            _is_backup: bool,
            _cursor: ScanCursor,
            _max_batch_cost: u64,
        ) -> anyhow::Result<ScanBatch> {
            Ok(ScanBatch::default())
        }

        fn is_loadable(&self, _key: &str) -> bool {
            true
        }

        fn pending_operation_count(&self) -> u64 {
            0
        }

        async fn soft_flush(&self) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn hard_flush(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn flush_key(
            &self,
            _map: &str,
            _key: &str,
            _value: &RecordValue,
            _is_backup: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn reset(&self) {}

        fn is_null(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn hard_flush_timeout_logs_warn_and_returns() {
        // The inner store sleeps 500ms per write — far longer than our 50ms
        // drain timeout — so hard_flush() will always exceed the deadline.
        let slow_inner: Arc<dyn MapDataStore> = Arc::new(SlowDataStore::new(500));
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            // Very short timeout so the test completes quickly.
            shutdown_timeout_ms: 50,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new(slow_inner, config);

        let val = dummy_value();
        store.add("slow_map", "key1", &val, 0, 1000).await.unwrap();
        store.add("slow_map", "key2", &val, 0, 1000).await.unwrap();

        assert_eq!(store.pending_operation_count(), 2);

        // hard_flush() must return within a window proportional to
        // shutdown_timeout_ms, not proportional to the slow inner-store delay.
        // The outer guard (500ms) is 10× the configured timeout (50ms) to
        // allow for scheduling jitter while still catching a pathological hang.
        let result =
            tokio::time::timeout(tokio::time::Duration::from_millis(500), store.hard_flush()).await;

        assert!(
            result.is_ok(),
            "hard_flush() must return within the outer timeout (should not hang)"
        );
        // hard_flush() itself should succeed (timeout is a warn, not an Err).
        assert!(
            result.unwrap().is_ok(),
            "hard_flush() should return Ok(()) on timeout, logging warnings"
        );
    }

    // -----------------------------------------------------------------------
    // WAL behavioral tests (AC1, AC2, AC7)
    // -----------------------------------------------------------------------

    use super::WalBootstrap;
    use crate::storage::wal::{Wal, WalEntry, WalFsyncPolicy};
    use std::sync::atomic::{AtomicU64 as TestAtomicU64, Ordering as TestOrdering};

    /// Minimal in-memory WAL for testing write-path WAL integration.
    struct InMemoryTestWal {
        appended: Arc<tokio::sync::Mutex<Vec<(u32, WalEntry)>>>,
        applied: Arc<tokio::sync::Mutex<Vec<(u32, u64)>>>,
        append_count: Arc<TestAtomicU64>,
    }

    impl InMemoryTestWal {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                appended: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                applied: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                append_count: Arc::new(TestAtomicU64::new(0)),
            })
        }
    }

    #[async_trait::async_trait]
    impl Wal for InMemoryTestWal {
        async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
            self.appended.lock().await.push((partition, entry.clone()));
            self.append_count.fetch_add(1, TestOrdering::Relaxed);
            Ok(())
        }

        async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
            self.applied.lock().await.push((partition, sequence));
            Ok(())
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            let guard = self.appended.lock().await;
            let applied_guard = self.applied.lock().await;
            let applied_seqs: std::collections::HashSet<u64> = applied_guard
                .iter()
                .filter(|(p, _)| *p == partition)
                .map(|(_, seq)| *seq)
                .collect();
            Ok(guard
                .iter()
                .filter(|(p, e)| *p == partition && !applied_seqs.contains(&e.sequence))
                .map(|(_, e)| e.clone())
                .collect())
        }
    }

    // AC1: WAL entry is appended before add() returns Ok(())
    #[tokio::test]
    async fn wal_append_happens_before_add_returns() {
        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        let val = dummy_value();
        store.add("m", "k1", &val, 0, 1000).await.unwrap();

        // WAL must contain the entry immediately after add() returns
        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(count, 1, "WAL must have exactly 1 entry after add()");

        let appended = wal.appended.lock().await;
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0].1.map, "m");
        assert_eq!(appended[0].1.key, "k1");
    }

    // AC1: WAL entry is appended before remove() returns Ok(())
    #[tokio::test]
    async fn wal_append_happens_before_remove_returns() {
        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        store.remove("m", "k1", 1000).await.unwrap();

        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(count, 1, "WAL must have exactly 1 entry after remove()");
    }

    // AC1: remove_all appends one WAL frame per key before returning Ok(())
    #[tokio::test]
    async fn wal_append_happens_before_remove_all_returns() {
        let wal = InMemoryTestWal::new();
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        let keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        store.remove_all("m", &keys).await.unwrap();

        // Every key must be durable in the WAL the moment remove_all() returns,
        // so an unclean crash replays each individual tombstone, not just the batch.
        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(
            count, 3,
            "WAL must have one entry per removed key after remove_all()"
        );

        let appended = wal.appended.lock().await;
        assert_eq!(appended.len(), 3);
        for entry in appended.iter() {
            assert_eq!(entry.1.map, "m");
            assert!(matches!(entry.1.op, WalOp::Remove));
        }
    }

    // AC2: after flush, WAL entry is marked applied
    #[tokio::test]
    async fn wal_entry_marked_applied_after_flush() {
        let wal = InMemoryTestWal::new();
        let applied_store = Arc::clone(&wal.applied);
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: wal_arc,
                sequence_start: 1,
            }),
        );

        let val = dummy_value();
        store.add("m", "k1", &val, 0, 1000).await.unwrap();

        // Wait for flush loop to process the entry
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        let applied = applied_store.lock().await;
        assert!(
            !applied.is_empty(),
            "WAL entry must be marked applied after the flush loop persists it to the inner store"
        );
    }

    /// Build a `from_source` lookup closure from a fixed map of overrides — the
    /// parallel-safe seam: full parse logic is exercised without touching
    /// process-global env, so this test cannot race any other test reading the
    /// same vars (the flake behind G3's withdrawal, TODO-468).
    fn config_source(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    // AC7: from_source parses TOPGUN_WAL_DIR and TOPGUN_WAL_FSYNC_POLICY
    #[test]
    fn from_source_parses_wal_dir_and_fsync_policy() {
        // TOPGUN_WAL_DIR
        let cfg =
            WriteBehindConfig::from_source(config_source(&[("TOPGUN_WAL_DIR", "/tmp/test-wal")]));
        assert_eq!(cfg.wal_dir, std::path::PathBuf::from("/tmp/test-wal"));

        // TOPGUN_WAL_FSYNC_POLICY with a valid value
        let cfg =
            WriteBehindConfig::from_source(config_source(&[("TOPGUN_WAL_FSYNC_POLICY", "per_op")]));
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);

        // The harness/doc `perop` spelling must resolve to PerOp (no silent
        // downgrade) now that the parser normalizes the separator/case.
        let cfg =
            WriteBehindConfig::from_source(config_source(&[("TOPGUN_WAL_FSYNC_POLICY", "perop")]));
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);

        // No override → Batched default
        let cfg = WriteBehindConfig::from_source(config_source(&[]));
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::Batched);
    }

    #[test]
    #[should_panic(expected = "is not a valid WAL fsync policy")]
    fn from_source_unknown_fsync_policy_is_fatal() {
        // An unknown durability policy must refuse to start rather than silently
        // degrade to the weaker default — the silent-downgrade path is what hid a
        // durability regression through a full RED soak.
        let _ = WriteBehindConfig::from_source(config_source(&[(
            "TOPGUN_WAL_FSYNC_POLICY",
            "invalid_policy",
        )]));
    }

    /// Wiring-only smoke test for the real `from_env` → `std::env` seam. The sole
    /// env-mutating config test, `#[serial]` so it cannot race the env-free tests
    /// above. Parse logic is covered by `from_source_*`; here we only prove
    /// `from_env` actually reads the process environment.
    #[serial_test::serial]
    #[test]
    fn from_env_reads_process_environment() {
        std::env::set_var("TOPGUN_WAL_FSYNC_POLICY", "per_op");
        let cfg = WriteBehindConfig::from_env();
        std::env::remove_var("TOPGUN_WAL_FSYNC_POLICY");
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);
    }

    // -----------------------------------------------------------------------
    // Buffer-aware read surface (scan/enumerate/list_maps overlay staging)
    // -----------------------------------------------------------------------

    use crate::storage::map_data_store::{MerkleLeaf, MerkleLeafKind};
    use crate::storage::record::OrMapEntry;
    use crate::storage::RedbDataStore;

    /// `WriteBehind` over a real redb inner with a flush delay long enough that
    /// `add`/`remove` stay buffered for the duration of a test (no background
    /// flush fires unless we force it via `flush_key`).
    fn redb_backed_store() -> (Arc<WriteBehindDataStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wb.redb");
        let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&path).expect("redb open"));
        let config = WriteBehindConfig {
            write_delay_ms: 600_000,
            flush_interval_ms: 600_000,
            capacity: 0,
            ..WriteBehindConfig::default()
        };
        (WriteBehindDataStore::new(inner, config), dir)
    }

    fn lww(value: &str, millis: u64) -> RecordValue {
        RecordValue::Lww {
            value: Value::String(value.to_string()),
            timestamp: Timestamp {
                millis,
                counter: 0,
                node_id: "n".to_string(),
            },
        }
    }

    /// Assert an `Lww` record carries the expected string value and HLC millis.
    /// `RecordValue` is intentionally not `PartialEq` (avoids a production derive
    /// just for tests), so destructure instead.
    fn assert_lww(record: &RecordValue, value: &str, millis: u64) {
        match record {
            RecordValue::Lww {
                value: Value::String(v),
                timestamp,
            } => {
                assert_eq!(v, value, "lww value mismatch");
                assert_eq!(timestamp.millis, millis, "lww hlc millis mismatch");
            }
            other => panic!("expected Lww string record, got {other:?}"),
        }
    }

    async fn drain_scan(
        store: &Arc<WriteBehindDataStore>,
        map: &str,
    ) -> Vec<(String, RecordValue)> {
        let mut out = Vec::new();
        let mut batch = store.scan_values(map, false, 0).await.unwrap();
        loop {
            out.append(&mut batch.records);
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched(map, false, cursor, 0)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }
        out
    }

    /// `LeafSink` that collects every emitted leaf for assertions.
    struct CollectLeaves {
        leaves: Vec<MerkleLeaf>,
    }

    #[async_trait]
    impl LeafSink for CollectLeaves {
        async fn consume(&mut self, batch: Vec<MerkleLeaf>) -> anyhow::Result<()> {
            self.leaves.extend(batch);
            Ok(())
        }
    }

    async fn collect_leaves(store: &Arc<WriteBehindDataStore>, map: &str) -> Vec<MerkleLeaf> {
        let mut sink = CollectLeaves { leaves: Vec::new() };
        store.enumerate_leaves(map, false, &mut sink).await.unwrap();
        sink.leaves
    }

    // AC2 — scan_values surfaces a buffered-only value; no double-count after
    // flush; a buffered write overrides an older flushed redb value.
    #[tokio::test]
    async fn ac2_scan_values_buffer_aware() {
        let (store, _dir) = redb_backed_store();

        // Buffered-only key: only in staging, never flushed.
        store.add("m", "k", &lww("v1", 1), 0, 1000).await.unwrap();
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "buffered-only key must be surfaced by scan");
        assert_eq!(rows[0].0, "k");
        assert_lww(&rows[0].1, "v1", 1);

        // Flush to redb; scan must still return it exactly once (no double-count).
        store
            .flush_key("m", "k", &lww("v1", 1), false)
            .await
            .unwrap();
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "flushed key must appear exactly once");
        assert_lww(&rows[0].1, "v1", 1);

        // A newer buffered write over the flushed redb value wins, exactly once.
        store.add("m", "k", &lww("v2", 5), 0, 2000).await.unwrap();
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "override must not double-count");
        assert_lww(&rows[0].1, "v2", 5);
    }

    // AC3 — a pending delete buffered in staging hides the flushed redb value
    // from both scan_values and enumerate_leaves (no resurrection).
    #[tokio::test]
    async fn ac3_pending_delete_suppression() {
        let (store, _dir) = redb_backed_store();

        store.add("m", "k", &lww("v1", 1), 0, 1000).await.unwrap();
        store
            .flush_key("m", "k", &lww("v1", 1), false)
            .await
            .unwrap();
        // Now buffer a delete (staging None) over the flushed value.
        store.remove("m", "k", 2000).await.unwrap();

        let rows = drain_scan(&store, "m").await;
        assert!(
            rows.is_empty(),
            "pending delete must hide the flushed value"
        );

        let leaves = collect_leaves(&store, "m").await;
        assert!(
            leaves.is_empty(),
            "pending delete must suppress the durable leaf"
        );
    }

    // AC4 — leaf-hash parity across flush + OrMap None-leaf propagation.
    #[tokio::test]
    async fn ac4_enumerate_leaves_hash_parity() {
        let (store, _dir) = redb_backed_store();

        // Buffered-only Lww leaf.
        store.add("m", "k", &lww("v1", 7), 0, 1000).await.unwrap();
        let buffered_leaves = collect_leaves(&store, "m").await;
        assert_eq!(buffered_leaves.len(), 1, "buffered-only leaf must surface");

        // Flush and re-enumerate: same leaf hash (Merkle root unchanged).
        store
            .flush_key("m", "k", &lww("v1", 7), false)
            .await
            .unwrap();
        let flushed_leaves = collect_leaves(&store, "m").await;
        assert_eq!(
            buffered_leaves, flushed_leaves,
            "leaf hash must be identical buffered vs flushed"
        );
        assert_eq!(buffered_leaves[0].kind, MerkleLeafKind::Lww);

        // OrMap sub-assertion: an OrMap entry yields a leaf; an OrTombstones
        // entry (merkle_leaf_hash == None) contributes NO leaf — no zero/
        // placeholder leaf injected.
        let or_value = RecordValue::OrMap {
            records: vec![OrMapEntry {
                value: Value::String("x".to_string()),
                tag: "t1".to_string(),
                timestamp: Timestamp {
                    millis: 1,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            }],
            tombstones: Vec::new(),
        };
        store.add("o", "ok", &or_value, 0, 1000).await.unwrap();
        let none_value = RecordValue::OrTombstones {
            tags: vec!["gone".to_string()],
        };
        store
            .add("o", "tombstoned", &none_value, 0, 1000)
            .await
            .unwrap();

        let or_leaves = collect_leaves(&store, "o").await;
        assert_eq!(
            or_leaves.len(),
            1,
            "OrMap entry yields a leaf; OrTombstones (None) contributes none"
        );
        assert_eq!(or_leaves[0].key, "ok");
        assert_eq!(or_leaves[0].kind, MerkleLeafKind::OrMap);
    }

    // AC5 — a backup scan/enumerate is byte-for-byte the inner result: no
    // overlay of non-backup staging writes.
    #[tokio::test]
    async fn ac5_is_backup_untouched() {
        let (store, _dir) = redb_backed_store();

        // Non-backup buffered write on map "m".
        store
            .add("m", "k", &lww("primary", 1), 0, 1000)
            .await
            .unwrap();

        // Backup scan must not see the non-backup staging write.
        let backup_batch = store.scan_values("m", true, 0).await.unwrap();
        assert!(
            backup_batch.records.is_empty(),
            "backup scan must not overlay non-backup staging writes"
        );

        let mut sink = CollectLeaves { leaves: Vec::new() };
        store.enumerate_leaves("m", true, &mut sink).await.unwrap();
        assert!(
            sink.leaves.is_empty(),
            "backup enumerate must not overlay non-backup staging writes"
        );
    }

    // AC6 — list_maps unions staging-only maps; pending-delete-only maps are
    // excluded.
    #[tokio::test]
    async fn ac6_list_maps_union() {
        let (store, _dir) = redb_backed_store();

        // Map whose only write is buffered, never flushed.
        store
            .add("buffered_map", "k", &lww("v", 1), 0, 1000)
            .await
            .unwrap();
        // Map present in staging only via a pending delete.
        store.remove("delete_only_map", "k", 1000).await.unwrap();

        let maps = store.list_maps().await.unwrap();
        assert!(
            maps.contains(&"buffered_map".to_string()),
            "buffered-only map must appear in list_maps"
        );
        assert!(
            !maps.contains(&"delete_only_map".to_string()),
            "pending-delete-only map must not appear in list_maps"
        );
    }

    // AC7 — multi-batch scan over flushed + staging-only keys returns the full
    // logical key set exactly once, no boundary miss, no staging-only duplicate.
    #[tokio::test]
    async fn ac7_cursor_safe_merge() {
        let (store, _dir) = redb_backed_store();

        // Flush several keys to redb so the scan spans multiple small batches.
        for i in 0..6u32 {
            let k = format!("flushed_{i:02}");
            let v = lww(&format!("v{i}"), 1);
            store.add("m", &k, &v, 0, 1000).await.unwrap();
            store.flush_key("m", &k, &v, false).await.unwrap();
        }
        // Add staging-only keys (buffered, never flushed) interleaved by name.
        for i in 0..4u32 {
            let k = format!("buffered_{i:02}");
            store.add("m", &k, &lww("b", 2), 0, 2000).await.unwrap();
        }

        // Force a tiny batch budget so the redb scan pages across many batches.
        let mut out: Vec<(String, RecordValue)> = Vec::new();
        let mut batch = store.scan_values("m", false, 1).await.unwrap();
        loop {
            out.append(&mut batch.records);
            match batch.next_cursor.take() {
                Some(cursor) => {
                    batch = store
                        .scan_values_batched("m", false, cursor, 1)
                        .await
                        .unwrap();
                }
                None => break,
            }
        }

        let mut keys: Vec<String> = out.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        let mut expected: Vec<String> = (0..6)
            .map(|i| format!("flushed_{i:02}"))
            .chain((0..4).map(|i| format!("buffered_{i:02}")))
            .collect();
        expected.sort();
        assert_eq!(
            keys, expected,
            "merged scan must yield every key exactly once across batches"
        );
    }

    // AC8 — the flush-vs-coalesce race must not drop read-your-writes. When a
    // flush dequeues an older write (E1) and a newer write (E2) coalesces into
    // staging before E1's terminal staging removal runs, that removal must NOT
    // wipe E2's slot. White-box: drive the exact race window deterministically
    // (the 600s flush interval keeps the background loop out of the way), then
    // assert the read surface still serves the newer value. Reproduces the
    // active-eviction soak residual (`expected=2 actual=1`) at unit scope.
    #[tokio::test]
    async fn ac8_flush_does_not_clear_newer_coalesced_staging() {
        let (store, _dir) = redb_backed_store();

        // E1: write v1. Staging now holds (seq1, v1); queue holds E1.
        store.add("m", "k", &lww("v1", 1), 0, 1000).await.unwrap();
        let pid = partition_for("m", "k");
        // Drain E1 out of the queue (the flush loop's first step). The queue lock
        // is now released, opening the race window.
        let drained = store.queues.get_mut(&pid).unwrap().drain_ready(i64::MAX);
        assert_eq!(drained.len(), 1, "E1 must be the only drained entry");
        let e1_seq = drained[0].sequence;

        // E2: a newer write coalesces in. Queue is empty (E1 already drained), so
        // E2 enters as a new entry; staging is overwritten to (seq2, v2).
        store.add("m", "k", &lww("v2", 5), 0, 2000).await.unwrap();

        // E1's flush completes and tries to clear its staging slot. With the seq
        // guard it must NOT remove the newer E2 slot.
        let removed = store.clear_staging_if_current("m", "k", e1_seq);
        assert!(
            !removed,
            "E1's terminal flush must not clear E2's newer staging slot"
        );

        // Read-your-writes holds: load and scan both surface v2, not the stale
        // durable/None value that the old unconditional remove exposed.
        assert_lww(&store.load("m", "k").await.unwrap().unwrap(), "v2", 5);
        let rows = drain_scan(&store, "m").await;
        assert_eq!(rows.len(), 1, "exactly one row for the key");
        assert_lww(&rows[0].1, "v2", 5);

        // When E2 itself flushes, its own seq clears the slot (no leak).
        let e2 = store.queues.get_mut(&pid).unwrap().drain_ready(i64::MAX);
        assert_eq!(e2.len(), 1, "E2 must be drained");
        let e2_seq = e2[0].sequence;
        assert!(
            store.clear_staging_if_current("m", "k", e2_seq),
            "E2's own flush clears its staging slot"
        );
        assert!(
            store.staging.is_empty(),
            "no staging slot may leak after both writes flush"
        );
    }

    // An out-of-order (lower-seq) stage must not clobber a newer staged value.
    // Two concurrent same-key writers can interleave between `next_sequence()`
    // and the staging insert; `stage` keeps the slot monotonic by `seq` so the
    // later write's value is the one that survives.
    #[tokio::test]
    async fn stage_is_monotonic_by_seq() {
        let (store, _dir) = redb_backed_store();

        store.stage("m", "k", 10, Some(lww("v10", 10)));
        // A higher seq replaces.
        store.stage("m", "k", 11, Some(lww("v11", 11)));
        assert_lww(&store.load("m", "k").await.unwrap().unwrap(), "v11", 11);
        // A late, lower seq is rejected (does not clobber the newer value).
        store.stage("m", "k", 10, Some(lww("v10", 10)));
        assert_lww(&store.load("m", "k").await.unwrap().unwrap(), "v11", 11);
        // Equal seq (same write, idempotent) is allowed.
        store.stage("m", "k", 11, None);
        assert!(
            store.load("m", "k").await.unwrap().is_none(),
            "equal-seq restage applies (idempotent same-write update)"
        );
    }
}

// ---------------------------------------------------------------------------
// Durability / crash-recovery tests (R12(b) WAL-retention + AC3f double-crash)
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "redb"))]
mod durability_tests {
    use super::*;
    use crate::storage::datastores::RedbDataStore;
    use crate::storage::wal::{WalRecovery, WalWriter};

    /// An OR-Map record carrying a single tombstone and no active records — the
    /// "element removed" state whose bytes the durability fence must never let a
    /// prune drop before they are durable.
    fn ormap_tombstone(tag: &str) -> RecordValue {
        RecordValue::OrMap {
            records: Vec::new(),
            tombstones: vec![tag.to_string()],
        }
    }

    fn buffered_config() -> WriteBehindConfig {
        // 60s delays so a write stays buffered (WAL-fsynced, not yet flushed to
        // the inner store) until we explicitly drain — the exact crash window.
        WriteBehindConfig {
            write_delay_ms: 60_000,
            flush_interval_ms: 60_000,
            shutdown_timeout_ms: 5_000,
            ..WriteBehindConfig::default()
        }
    }

    /// `R12(b)`: an un-flushed frame is NEVER discarded — a tombstone's bytes are
    /// always in the inner store OR still in the WAL, never both-lost. A WAL-GC
    /// change that deleted an unapplied frame would break this guard.
    #[tokio::test]
    async fn r12b_wal_retains_unflushed_tombstone_frame_until_applied() {
        let wal_dir = tempfile::tempdir().unwrap();
        let redb_dir = tempfile::tempdir().unwrap();
        let inner: Arc<dyn MapDataStore> =
            Arc::new(RedbDataStore::new(redb_dir.path().join("d.redb")).unwrap());
        let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&inner),
            buffered_config(),
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );

        store
            .add("m", "k1", &ormap_tombstone("T1"), 0, 1)
            .await
            .unwrap();
        let partition = partition_for("m", "k1");

        // Buffered: the WAL frame is fsynced but the inner store has not seen it.
        // R12(b) requires the WAL to still hold the frame (never both-lost).
        assert!(
            !wal.unapplied(partition).await.unwrap().is_empty(),
            "un-flushed tombstone frame must be retained in the WAL"
        );
        assert!(
            inner.load("m", "k1").await.unwrap().is_none(),
            "the buffered tombstone is not yet in the inner store"
        );

        // Drain to the inner store; only NOW is the frame eligible for release.
        store.hard_flush().await.unwrap();
        assert!(
            wal.unapplied(partition).await.unwrap().is_empty(),
            "once the bytes are durable in the inner store the frame is applied/releasable"
        );
        assert!(
            matches!(
                inner.load("m", "k1").await.unwrap(),
                Some(RecordValue::OrMap { tombstones, .. }) if tombstones == vec!["T1".to_string()]
            ),
            "tombstone bytes are durable in the inner store after the drain"
        );
    }

    /// `AC3f`: crash after WAL-fsync but before inner-store flush, recover, crash
    /// AGAIN during recovery (before the frame is marked applied) — the tombstone
    /// bytes are recoverable at every step (WAL retention held) and no removed
    /// element is resurrected.
    #[tokio::test]
    async fn ac3f_double_crash_wal_retention_recovers_tombstone_no_resurrection() {
        let wal_dir = tempfile::tempdir().unwrap();
        let redb_dir = tempfile::tempdir().unwrap();
        let redb_path = redb_dir.path().join("d.redb");
        let partition = partition_for("m", "k1");

        // --- Crash 1: after WAL fsync, before inner-store flush ---
        {
            // Emulate the write path's WAL-append-before-ack for a tombstone,
            // then a kill -9 STRICTLY before the background flush reaches the inner
            // store: fsync the frame, drop the writer, touch no inner store. (Going
            // through WriteBehindDataStore here would leave its background flush task
            // holding the inner-store handle — a real crash releases it; the direct
            // append is the faithful "frame fsynced, inner never written" window.)
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
            let entry = WalEntry {
                map: "m".to_string(),
                key: "k1".to_string(),
                op: WalOp::Store {
                    value: WalStorePayload::Record(ormap_tombstone("T1")),
                    expiration_time: None,
                },
                timestamp: None,
                sequence: 1,
            };
            wal.append(partition, &entry).await.unwrap();
        }

        // --- Crash 2: DURING recovery — replay the still-unapplied frame into the
        // inner store, then "die" before marking it applied. ---
        {
            let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&redb_path).unwrap());
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
            let unapplied = wal.unapplied(partition).await.unwrap();
            assert!(
                !unapplied.is_empty(),
                "the un-flushed tombstone frame survived crash 1 in the WAL (R12(b))"
            );
            for entry in &unapplied {
                if let WalOp::Store {
                    value: WalStorePayload::Record(rv),
                    expiration_time,
                } = &entry.op
                {
                    let exp = expiration_time.unwrap_or(0);
                    inner.add(&entry.map, &entry.key, rv, exp, 1).await.unwrap();
                }
            }
            // Process "crashes" HERE — no mark_applied.
            assert!(
                matches!(
                    inner.load("m", "k1").await.unwrap(),
                    Some(RecordValue::OrMap { tombstones, .. }) if tombstones == vec!["T1".to_string()]
                ),
                "tombstone recovered into the inner store mid-recovery"
            );
        }

        // --- Recovery completes: crash 2 never marked the frame applied, so R12(b)
        // still holds it; a full WalRecovery replays it again (idempotent) and
        // finally marks it applied. The tombstone survives BOTH crashes. ---
        {
            let inner: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(&redb_path).unwrap());
            let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
            assert!(
                !wal.unapplied(partition).await.unwrap().is_empty(),
                "crash 2 did not mark the frame applied, so WAL retention still holds it"
            );
            let recovery = WalRecovery::new(Arc::clone(&wal), Vec::new());
            recovery.run(Arc::clone(&inner)).await.unwrap();

            match inner.load("m", "k1").await.unwrap() {
                Some(RecordValue::OrMap {
                    records,
                    tombstones,
                }) => {
                    assert_eq!(
                        tombstones,
                        vec!["T1".to_string()],
                        "tombstone present after both crashes — the removal is preserved"
                    );
                    assert!(
                        records.is_empty(),
                        "no active record resurrected for the removed element"
                    );
                }
                other => {
                    panic!("expected the OR-Map tombstone to survive both crashes, got {other:?}")
                }
            }
            assert!(
                wal.unapplied(partition).await.unwrap().is_empty(),
                "a completed recovery marks the frame applied"
            );
        }
    }
}
