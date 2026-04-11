//! Write-behind buffering layer for [`MapDataStore`].
//!
//! Decouples mutation latency from persistence latency by buffering writes
//! in per-partition coalesced queues and flushing them on a configurable schedule.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{watch, Notify};
use topgun_core::{fnv1a_hash, PARTITION_COUNT};
use tracing::warn;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the write-behind buffering layer.
///
/// Controls flush timing, batch sizes, retry behavior, and capacity limits.
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
        }
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
        self.entries
            .remove(&(map.to_string(), key.to_string()))
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
    /// Shutdown signal sender for the background flush task.
    _shutdown: watch::Sender<bool>,
}

impl WriteBehindDataStore {
    /// Creates a new write-behind data store wrapping `inner`.
    ///
    /// Spawns a background flush task that runs until the returned `Arc` (and
    /// all clones) are dropped or shutdown is signalled.
    pub fn new(inner: Arc<dyn MapDataStore>, config: WriteBehindConfig) -> Arc<Self> {
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
            _shutdown: shutdown_tx,
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
                            .add(&entry.map, &entry.key, value, *expiration_time, entry.store_time)
                            .await
                    }
                    DelayedOp::Remove => {
                        store
                            .inner
                            .remove(&entry.map, &entry.key, entry.store_time)
                            .await
                    }
                };

                match result {
                    Ok(()) => {
                        // Successfully flushed -- remove from staging and decrement count
                        store
                            .staging
                            .remove(&(entry.map.clone(), entry.key.clone()));
                        store.pending_count.fetch_sub(1, Ordering::Relaxed);
                    }
                    Err(err) => {
                        let new_retry = entry.retry_count + 1;
                        if new_retry < store.config.max_retries {
                            // Reinsert with incremented retry count
                            let mut retry_entry = entry.clone();
                            retry_entry.retry_count = new_retry;

                            let partition_id = partition_for(&entry.map, &entry.key);
                            let mut queue = store
                                .queues
                                .entry(partition_id)
                                .or_default();
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
        // Check capacity before insertion to avoid partial state on rejection
        if self.config.capacity != 0 {
            let current = self.pending_count.load(Ordering::Relaxed);
            // Only reject if this would be a NEW key (not a coalesce).
            // We check the staging area as a fast pre-check; the definitive
            // check happens via PartitionQueue::insert return value.
            let is_new_key = !self.staging.contains_key(&(map.to_string(), key.to_string()));
            if is_new_key && current >= self.config.capacity {
                anyhow::bail!("Write-behind capacity exceeded");
            }
        }

        let partition_id = partition_for(map, key);
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
        };

        // Insert into partition queue, preserving original store_time on coalesce
        let mut queue = self
            .queues
            .entry(partition_id)
            .or_default();

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
        // Check capacity before insertion
        if self.config.capacity != 0 {
            let current = self.pending_count.load(Ordering::Relaxed);
            let is_new_key = !self.staging.contains_key(&(map.to_string(), key.to_string()));
            if is_new_key && current >= self.config.capacity {
                anyhow::bail!("Write-behind capacity exceeded");
            }
        }

        let partition_id = partition_for(map, key);
        let entry = DelayedEntry {
            map: map.to_string(),
            key: key.to_string(),
            operation: DelayedOp::Remove,
            store_time: now,
            sequence: self.next_sequence(),
            retry_count: 0,
        };

        let mut queue = self
            .queues
            .entry(partition_id)
            .or_default();

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
        let now = now_millis();

        for key in keys {
            // Check capacity before each insertion
            if self.config.capacity != 0 {
                let current = self.pending_count.load(Ordering::Relaxed);
                let is_new_key =
                    !self.staging.contains_key(&(map.to_string(), key.clone()));
                if is_new_key && current >= self.config.capacity {
                    anyhow::bail!("Write-behind capacity exceeded");
                }
            }

            let partition_id = partition_for(map, key);
            let entry = DelayedEntry {
                map: map.to_string(),
                key: key.clone(),
                operation: DelayedOp::Remove,
                store_time: now,
                sequence: self.next_sequence(),
                retry_count: 0,
            };

            let mut queue = self
                .queues
                .entry(partition_id)
                .or_default();

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
        // Loop until all pending entries are drained, with a 30-second timeout
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);

        loop {
            if self.pending_count.load(Ordering::Relaxed) == 0 {
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "hard_flush timed out after 30s with {} entries remaining",
                    self.pending_count.load(Ordering::Relaxed)
                );
            }

            // Signal the flush loop and yield briefly
            self.flush_notify.notify_one();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
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
        }
    }

    // -----------------------------------------------------------------------
    // SpyDataStore
    // -----------------------------------------------------------------------

    /// Records which calls were made to the inner store, for test assertions.
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    enum SpyCall {
        Add {
            map: String,
            key: String,
        },
        Remove {
            map: String,
            key: String,
        },
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
                if let Some(v) = self
                    .seeded
                    .get(&(map.to_string(), key.clone()))
                {
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
        assert!(loaded.is_some(), "Staged value should be returned by load()");
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
            recorded.iter().any(|c| matches!(c, SpyCall::Add { map, key } if map == "map1" && key == "key1")),
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
        assert!(
            result.is_err(),
            "Should reject writes exceeding capacity"
        );

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
            store.staging.get(&("map1".to_string(), "key1".to_string())).is_none(),
            "Staging should be cleared after flush_key"
        );

        // Pending count should be decremented
        assert_eq!(store.pending_operation_count(), 0);

        // Inner store should have received an add call
        let recorded = calls.lock().unwrap();
        assert!(
            recorded.iter().any(|c| matches!(c, SpyCall::Add { map, key } if map == "map1" && key == "key1")),
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
}
