//! Write-behind buffering layer for [`MapDataStore`].
//!
//! Decouples mutation latency from persistence latency by buffering writes
//! in per-partition coalesced queues and flushing them on a configurable schedule.
//! An optional WAL ensures every acked write is durable before the ack is returned.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{watch, Notify};
use topgun_core::{fnv1a_hash, PARTITION_COUNT};
use tracing::warn;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;
use crate::storage::wal::{Wal, WalEntry, WalFsyncPolicy, WalOp};

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
    /// `Batched` (the default) amortises fsync cost with group commit (~10ms/~100 ops).
    /// Do NOT change to `PerOp` in production without evaluating write latency impact.
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
        let defaults = Self::default();
        let mut cfg = defaults.clone();

        // Parse TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS → flush_interval_ms (u64)
        if let Ok(raw) = std::env::var("TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS") {
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
        if let Ok(raw) = std::env::var("TOPGUN_WRITEBEHIND_BATCH_SIZE") {
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
        if let Ok(raw) = std::env::var("TOPGUN_WRITEBEHIND_CAPACITY") {
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
        if let Ok(raw) = std::env::var("TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS") {
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
        if let Ok(raw) = std::env::var("TOPGUN_WAL_DIR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                cfg.wal_dir = PathBuf::from(trimmed);
            }
        }

        // Parse TOPGUN_WAL_FSYNC_POLICY → wal_fsync_policy (WalFsyncPolicy)
        if let Ok(raw) = std::env::var("TOPGUN_WAL_FSYNC_POLICY") {
            match raw.trim().parse::<WalFsyncPolicy>() {
                Ok(policy) => {
                    cfg.wal_fsync_policy = policy;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "topgun_server::storage::write_behind",
                        var = "TOPGUN_WAL_FSYNC_POLICY",
                        value = %raw,
                        error = %err,
                        default = ?defaults.wal_fsync_policy,
                        "Failed to parse env var; using default"
                    );
                    cfg.wal_fsync_policy = defaults.wal_fsync_policy;
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
    /// `Some(value)` = pending write, `None` = pending delete.
    staging: DashMap<(String, String), Option<RecordValue>>,
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
}

impl WriteBehindDataStore {
    /// Creates a new write-behind data store wrapping `inner`.
    ///
    /// Spawns a background flush task that runs until the returned `Arc` (and
    /// all clones) are dropped or shutdown is signalled.
    pub fn new(inner: Arc<dyn MapDataStore>, config: WriteBehindConfig) -> Arc<Self> {
        Self::new_with_wal(inner, config, None)
    }

    /// Creates a new write-behind data store with an optional WAL.
    ///
    /// When `wal` is `Some`, every write is appended to the WAL and satisfies
    /// the configured fsync policy before returning `Ok(())`. When `None`, the
    /// store behaves identically to `new` (no crash-safe durability guarantee).
    pub fn new_with_wal(
        inner: Arc<dyn MapDataStore>,
        config: WriteBehindConfig,
        wal: Option<Arc<dyn Wal>>,
    ) -> Arc<Self> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let flush_notify = Arc::new(Notify::new());

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
            wal_sequence: AtomicU64::new(0),
        });

        // Spawn background flush loop with a clone of the Arc
        let store_clone = Arc::clone(&store);
        tokio::spawn(flush_loop(store_clone, shutdown_rx));

        store
    }

    /// Returns the next sequence number for ordering.
    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed)
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
                        // Successfully flushed -- remove from staging and decrement count
                        store
                            .staging
                            .remove(&(entry.map.clone(), entry.key.clone()));
                        store.pending_count.fetch_sub(1, Ordering::Relaxed);
                        // Mark the WAL entry applied so a clean restart does not
                        // re-replay writes that are already durable in the inner store.
                        if let Some(wal) = &store.wal {
                            if let Err(err) = wal.mark_applied(partition_id_for_wal, entry.wal_sequence).await {
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
                            store
                                .staging
                                .remove(&(entry.map.clone(), entry.key.clone()));
                            store.pending_count.fetch_sub(1, Ordering::Relaxed);
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
        let wal_value_for_entry = match value {
            RecordValue::Lww { value: v, timestamp } => {
                let wal_op = WalOp::Store {
                    value: v.clone(),
                    expiration_time: if expiration_time == 0 { None } else { Some(expiration_time) },
                };
                let wal_ts = Some(timestamp.clone());
                (wal_op, wal_ts)
            }
            _ => {
                // Non-LWW values use a Store op with no timestamp.
                let wal_op = WalOp::Store {
                    value: topgun_core::types::Value::Null,
                    expiration_time: if expiration_time == 0 { None } else { Some(expiration_time) },
                };
                (wal_op, None)
            }
        };
        let wal_entry = WalEntry {
            map: map.to_string(),
            key: key.to_string(),
            op: wal_value_for_entry.0,
            timestamp: wal_value_for_entry.1,
            sequence: wal_seq,
        };
        self.wal_append(partition_id, &wal_entry).await?;

        let entry = DelayedEntry {
            map: map.to_string(),
            key: key.to_string(),
            operation: DelayedOp::Store {
                value: value.clone(),
                expiration_time,
            },
            store_time: now,
            sequence: self.next_sequence(),
            retry_count: 0,
            wal_sequence: wal_seq,
        };

        // Insert into partition queue, preserving original store_time on coalesce
        let mut queue = self.queues.entry(partition_id).or_default();

        // If coalescing, preserve the original store_time
        let staging_key = (map.to_string(), key.to_string());
        if let Some(existing) = queue.value_mut().remove(map, key) {
            let mut coalesced = entry;
            coalesced.store_time = existing.store_time;
            let _ = queue.value_mut().insert(coalesced);
            // No pending_count change on coalesce
        } else {
            let _ = queue.value_mut().insert(entry);
            // New key -- increment pending count
            self.pending_count.fetch_add(1, Ordering::Relaxed);
        }

        // Update staging area for read-your-writes
        self.staging.insert(staging_key, Some(value.clone()));

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

        let entry = DelayedEntry {
            map: map.to_string(),
            key: key.to_string(),
            operation: DelayedOp::Remove,
            store_time: now,
            sequence: self.next_sequence(),
            retry_count: 0,
            wal_sequence: wal_seq,
        };

        let mut queue = self.queues.entry(partition_id).or_default();

        let staging_key = (map.to_string(), key.to_string());
        if let Some(existing) = queue.value_mut().remove(map, key) {
            let mut coalesced = entry;
            coalesced.store_time = existing.store_time;
            let _ = queue.value_mut().insert(coalesced);
            // No pending_count change on coalesce
        } else {
            let _ = queue.value_mut().insert(entry);
            self.pending_count.fetch_add(1, Ordering::Relaxed);
        }

        // Pending delete marker in staging
        self.staging.insert(staging_key, None);

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
            return match entry.value() {
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
                if let Some(value) = entry.value() {
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

            let entry = DelayedEntry {
                map: map.to_string(),
                key: key.clone(),
                operation: DelayedOp::Remove,
                store_time: now,
                sequence: self.next_sequence(),
                retry_count: 0,
                wal_sequence: wal_seq,
            };

            let mut queue = self.queues.entry(partition_id).or_default();

            let staging_key = (map.to_string(), key.clone());
            if let Some(existing) = queue.value_mut().remove(map, key) {
                let mut coalesced = entry;
                coalesced.store_time = existing.store_time;
                let _ = queue.value_mut().insert(coalesced);
            } else {
                let _ = queue.value_mut().insert(entry);
                self.pending_count.fetch_add(1, Ordering::Relaxed);
            }

            self.staging.insert(staging_key, None);
        }

        Ok(())
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
                        // Mark WAL applied so a restart after clean shutdown is
                        // a no-op rather than re-replaying already-durable writes.
                        if let Some(wal) = &self.wal {
                            if let Err(err) = wal.mark_applied(partition_id_for_wal, entry.wal_sequence).await {
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

        // Decrement pending count if the key was actually in the queue
        if removed.is_some() {
            self.pending_count.fetch_sub(1, Ordering::Relaxed);
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
        self.inner.reset();
    }

    fn is_null(&self) -> bool {
        false
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
        let store = WriteBehindDataStore::new_with_wal(inner, config, Some(wal_arc));

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
        let store = WriteBehindDataStore::new_with_wal(inner, config, Some(wal_arc));

        store.remove("m", "k1", 1000).await.unwrap();

        let count = wal.append_count.load(TestOrdering::Relaxed);
        assert_eq!(count, 1, "WAL must have exactly 1 entry after remove()");
    }

    // AC2: after flush, WAL entry is marked applied
    #[tokio::test]
    async fn wal_entry_marked_applied_after_flush() {
        let wal = InMemoryTestWal::new();
        let applied_store = Arc::clone(&wal.applied);
        let wal_arc: Arc<dyn Wal> = Arc::clone(&wal) as Arc<dyn Wal>;

        let inner: Arc<dyn MapDataStore> = Arc::new(SpyDataStore::new());
        let config = short_delay_config();
        let store = WriteBehindDataStore::new_with_wal(inner, config, Some(wal_arc));

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

    // AC7: from_env parses TOPGUN_WAL_DIR and TOPGUN_WAL_FSYNC_POLICY
    #[test]
    fn from_env_parses_wal_dir_and_fsync_policy() {
        // Test TOPGUN_WAL_DIR
        std::env::set_var("TOPGUN_WAL_DIR", "/tmp/test-wal");
        let cfg = WriteBehindConfig::from_env();
        assert_eq!(cfg.wal_dir, std::path::PathBuf::from("/tmp/test-wal"));
        std::env::remove_var("TOPGUN_WAL_DIR");

        // Test TOPGUN_WAL_FSYNC_POLICY with valid value
        std::env::set_var("TOPGUN_WAL_FSYNC_POLICY", "per_op");
        let cfg = WriteBehindConfig::from_env();
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::PerOp);
        std::env::remove_var("TOPGUN_WAL_FSYNC_POLICY");

        // Test TOPGUN_WAL_FSYNC_POLICY with invalid value falls back to Batched
        std::env::set_var("TOPGUN_WAL_FSYNC_POLICY", "invalid_policy");
        let cfg = WriteBehindConfig::from_env();
        assert_eq!(
            cfg.wal_fsync_policy,
            WalFsyncPolicy::Batched,
            "Invalid policy must fall back to Batched default"
        );
        std::env::remove_var("TOPGUN_WAL_FSYNC_POLICY");

        // Default is Batched
        let cfg = WriteBehindConfig::default();
        assert_eq!(cfg.wal_fsync_policy, WalFsyncPolicy::Batched);
    }
}
