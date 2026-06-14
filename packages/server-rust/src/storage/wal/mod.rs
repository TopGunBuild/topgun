//! Write-Ahead Log (WAL) trait surface, concrete writer, and recovery.
//!
//! Defines the frozen contract and implements the production file-backed
//! writer (`WalWriter`) and startup recovery (`WalRecovery`).
//!
//! The WAL is per-partition: one append-only file per partition under a
//! configured `wal_dir`. `WalWriter` implements the `Wal` trait; `WalRecovery`
//! replays un-applied entries through the inner store on startup.

pub mod format;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

// ---------------------------------------------------------------------------
// WalFsyncPolicy
// ---------------------------------------------------------------------------

/// Controls how aggressively the WAL writer calls `fsync` after writing frames.
///
/// Choosing the right policy is a crash-safety vs. throughput tradeoff:
/// - `PerOp` maximises durability at the cost of per-write syscall overhead.
/// - `Batched` amortises fsync cost across a group of writes (the default).
/// - `None` skips fsync entirely — useful for tests and throughput benchmarks
///   where crash-safety is not required.
///
/// The *behaviour* of the policy (actually calling fsync) is implemented in the
/// `WalWriter` (307c). This enum is the configuration carrier only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WalFsyncPolicy {
    /// Call `fsync` after every appended frame. Safest; highest latency.
    PerOp,
    /// Call `fsync` after each flush batch. Default — good balance of safety
    /// and throughput.
    #[default]
    Batched,
    /// Never call `fsync`. OS-buffered writes only. Not crash-safe.
    None,
}

/// Parsing error returned when an unknown policy string is encountered.
///
/// The env-parse contract is: unknown values are rejected rather than silently
/// defaulted, so a misconfigured deployment surfaces an error at startup instead
/// of quietly running with weaker durability guarantees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWalFsyncPolicyError(pub String);

impl std::fmt::Display for ParseWalFsyncPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown WAL fsync policy {:?}; valid values are per_op, batched, none",
            self.0
        )
    }
}

impl std::error::Error for ParseWalFsyncPolicyError {}

impl FromStr for WalFsyncPolicy {
    type Err = ParseWalFsyncPolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "per_op" => Ok(Self::PerOp),
            "batched" => Ok(Self::Batched),
            "none" => Ok(Self::None),
            other => Err(ParseWalFsyncPolicyError(other.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// WalOp — op enum mirroring DelayedOp
// ---------------------------------------------------------------------------

/// The operation recorded in a WAL entry, mirroring the write-behind `DelayedOp`
/// shape so that WAL-driven recovery can replay the exact same operation.
///
/// Using a typed enum rather than a string tag avoids the silent type-mismatch
/// class of bugs and lets serde enforce the variant structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WalOp {
    /// Upsert: the record value and its TTL expiration timestamp in milliseconds.
    Store {
        /// The full CRDT value to persist.
        value: Value,
        /// Wall-clock expiration time in milliseconds since epoch.
        /// Negative or zero means no expiration.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        expiration_time: Option<i64>,
    },
    /// Tombstone: remove the record from the store.
    Remove,
}

// ---------------------------------------------------------------------------
// WalEntry
// ---------------------------------------------------------------------------

/// A single record in the Write-Ahead Log.
///
/// Each entry captures the map, key, operation, HLC timestamp (for idempotent
/// replay deduplication), and a monotonic sequence number for ordering.
///
/// The HLC `Timestamp` is copied directly from `RecordValue::Lww { timestamp }`
/// so that recovery can skip entries that have already been superseded by a
/// higher-timestamped write — no new field on `RecordValue` is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalEntry {
    /// Map name this entry belongs to.
    pub map: String,
    /// Record key within the map.
    pub key: String,
    /// The operation to replay on recovery.
    pub op: WalOp,
    /// HLC timestamp from the originating write. Used as the idempotency key:
    /// during recovery, a replayed entry whose timestamp is older than the
    /// current in-memory value is a no-op.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timestamp: Option<Timestamp>,
    /// Monotonically increasing counter assigned at append time. Establishes
    /// total ordering within a partition WAL file for ordered replay.
    pub sequence: u64,
}

// ---------------------------------------------------------------------------
// Wal trait
// ---------------------------------------------------------------------------

/// Object-safe WAL interface.
///
/// Both the production file-backed `WalWriter` (307c) and the in-memory
/// simulation double (307d) implement this trait so callers can depend on
/// `Arc<dyn Wal>` without coupling to a specific implementation.
///
/// All methods are async because the production implementation performs I/O.
/// The fsync behaviour is governed by the `WalFsyncPolicy` that each
/// implementation is configured with — callers do not call fsync directly.
#[async_trait]
pub trait Wal: Send + Sync {
    /// Appends a `WalEntry` to the log for the given partition.
    ///
    /// The implementation is responsible for encoding the entry, writing it to
    /// the appropriate per-partition file, and fsyncing according to the
    /// configured `WalFsyncPolicy`. Returns an error if the write or fsync
    /// fails; the caller should NOT ack the client until this returns `Ok`.
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()>;

    /// Marks a WAL entry as applied and removes it from the set of entries
    /// that `unapplied` would return.
    ///
    /// Called after the entry has been successfully flushed to the durable
    /// inner store. The implementation may truncate or checkpoint the log
    /// when all entries up to a sequence number have been marked applied.
    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()>;

    /// Returns all un-applied entries for a partition, in sequence order.
    ///
    /// Called at startup (307c recovery loop) to replay any entries that were
    /// appended before the last crash but not yet acknowledged as applied.
    /// An empty vec means the partition is clean.
    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>>;
}

// ===========================================================================
// WalWriter — production file-backed implementation
// ===========================================================================

/// Per-partition file handle managed by `WalWriter`.
struct PartitionFile {
    file: tokio::fs::File,
    /// How many frames have been written since the last fsync (for batched policy).
    unfsynced_count: u32,
}

/// Production append-only WAL writer.
///
/// Maintains one file per partition under `wal_dir` named
/// `partition-{id:03}.log`. Applied-sequence tracking uses a sidecar file
/// `partition-{id:03}.applied` that stores the highest applied sequence number
/// as a big-endian u64 so recovery can skip already-durable entries.
///
/// Thread safety: each partition file is guarded by its own `AsyncMutex` so
/// concurrent appends to different partitions do not block each other.
pub struct WalWriter {
    wal_dir: PathBuf,
    policy: WalFsyncPolicy,
    /// Per-partition file handles, lazily opened on first append.
    files: AsyncMutex<HashMap<u32, PartitionFile>>,
    /// Batched-commit group-commit timer: shared notifier woken every ~10 ms.
    /// When `policy == Batched`, the writer also fsyncs after every 100 ops.
    batch_flush_tx: tokio::sync::watch::Sender<()>,
}

impl WalWriter {
    /// Creates a new `WalWriter` rooted at `wal_dir`.
    ///
    /// The directory is created if it does not exist. Returns an error if the
    /// directory cannot be created, so a misconfigured `wal_dir` surfaces at
    /// startup rather than on the first write.
    ///
    /// # Errors
    ///
    /// Returns an error if `wal_dir` cannot be created or accessed.
    pub fn new(wal_dir: PathBuf, policy: WalFsyncPolicy) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&wal_dir).map_err(|e| {
            anyhow::anyhow!(
                "Cannot create WAL directory {}: {e}. \
                 Ensure the WAL dir is on the same volume as the durable store.",
                wal_dir.display()
            )
        })?;

        let (batch_flush_tx, batch_flush_rx) = tokio::sync::watch::channel(());

        let writer = Arc::new(Self {
            wal_dir,
            policy,
            files: AsyncMutex::new(HashMap::new()),
            batch_flush_tx,
        });

        // For the Batched policy, spawn a background task that fsyncs all open
        // partition files every ~10 ms so group-commit amortises fsync cost.
        if policy == WalFsyncPolicy::Batched {
            let writer_weak = Arc::downgrade(&writer);
            tokio::spawn(async move {
                let mut rx = batch_flush_rx;
                let interval = tokio::time::Duration::from_millis(10);
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(interval) => {}
                        result = rx.changed() => {
                            if result.is_err() {
                                return; // sender dropped — writer gone
                            }
                        }
                    }
                    let Some(w) = writer_weak.upgrade() else {
                        return;
                    };
                    let mut guard = w.files.lock().await;
                    for pf in guard.values_mut() {
                        if pf.unfsynced_count > 0 {
                            let _ = pf.file.sync_data().await;
                            pf.unfsynced_count = 0;
                        }
                    }
                }
            });
        }

        Ok(writer)
    }

    /// Path of the append log for a partition.
    fn log_path(&self, partition: u32) -> PathBuf {
        self.wal_dir.join(format!("partition-{partition:03}.log"))
    }

    /// Path of the sidecar file that records the highest applied sequence.
    fn applied_path(&self, partition: u32) -> PathBuf {
        self.wal_dir
            .join(format!("partition-{partition:03}.applied"))
    }

    /// Reads the highest applied sequence number for a partition from the
    /// sidecar file, or 0 if the file does not exist.
    fn read_applied_sequence(path: &Path) -> u64 {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() == 8 => u64::from_be_bytes(bytes.try_into().unwrap_or([0; 8])),
            _ => 0,
        }
    }

    /// Writes the highest applied sequence to the sidecar file atomically
    /// (write to `.tmp` then rename).
    fn write_applied_sequence(path: &Path, seq: u64) -> anyhow::Result<()> {
        let tmp = path.with_extension("applied.tmp");
        std::fs::write(&tmp, seq.to_be_bytes())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[async_trait]
impl Wal for WalWriter {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        let frame = format::encode(entry)?;

        let mut files = self.files.lock().await;
        let pf = if let Some(pf) = files.get_mut(&partition) {
            pf
        } else {
            let path = self.log_path(partition);
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|e| anyhow::anyhow!("Cannot open WAL file {}: {e}", path.display()))?;
            files.insert(
                partition,
                PartitionFile {
                    file,
                    unfsynced_count: 0,
                },
            );
            files.get_mut(&partition).unwrap()
        };

        pf.file
            .write_all(&frame)
            .await
            .map_err(|e| anyhow::anyhow!("WAL write failed for partition {partition}: {e}"))?;

        // Push tokio's internal write buffer out to the OS so a subsequent fresh
        // read of this partition file (unapplied/recovery/compaction all reopen
        // the path) observes the entry. This is visibility, not durability:
        // flush() does not fsync. Durability is governed separately by the policy
        // match below. Without this, the None policy never leaves tokio's buffer
        // and a concurrent reader can miss a just-appended entry.
        pf.file
            .flush()
            .await
            .map_err(|e| anyhow::anyhow!("WAL flush failed for partition {partition}: {e}"))?;

        match self.policy {
            WalFsyncPolicy::PerOp => {
                pf.file.sync_data().await.map_err(|e| {
                    anyhow::anyhow!("WAL fsync failed for partition {partition}: {e}")
                })?;
                pf.unfsynced_count = 0;
            }
            WalFsyncPolicy::Batched => {
                pf.unfsynced_count += 1;
                // Flush immediately when the batch threshold is reached so
                // high-throughput bursts don't wait for the timer task.
                if pf.unfsynced_count >= 100 {
                    pf.file.sync_data().await.map_err(|e| {
                        anyhow::anyhow!("WAL batch fsync failed for partition {partition}: {e}")
                    })?;
                    pf.unfsynced_count = 0;
                    // Notify the background timer that we just fsynced so it
                    // resets its internal cooldown.
                    let _ = self.batch_flush_tx.send(());
                }
            }
            WalFsyncPolicy::None => {
                // OS-buffered only; no fsync call.
            }
        }

        Ok(())
    }

    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        let path = self.applied_path(partition);
        // Read the current max and only advance it, never regress it.
        let current = Self::read_applied_sequence(&path);
        if sequence > current {
            Self::write_applied_sequence(&path, sequence).map_err(|e| {
                anyhow::anyhow!("Cannot write applied sidecar {}: {e}", path.display())
            })?;
        }

        // After marking applied, compact the WAL log: rebuild it with only
        // entries whose sequence is above the new applied watermark.
        self.compact_log(partition, sequence).await
    }

    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        let log_path = self.log_path(partition);
        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let applied_seq = Self::read_applied_sequence(&self.applied_path(partition));

        let data = tokio::fs::read(&log_path)
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read WAL file {}: {e}", log_path.display()))?;

        let entries = match format::decode_all(&data) {
            format::FrameDecodeResult::Complete(entries) => entries,
            format::FrameDecodeResult::CleanEof => Vec::new(),
            format::FrameDecodeResult::TruncatedTail { complete } => {
                tracing::warn!(
                    partition = partition,
                    path = ?log_path,
                    "WAL tail truncated (crash mid-write); replaying intact prefix"
                );
                complete
            }
            format::FrameDecodeResult::BadMagic { offset } => {
                let path_display = log_path.display();
                anyhow::bail!(
                    "WAL file {path_display} has wrong magic at offset {offset}; \
                     refusing to start to avoid replaying corrupt data"
                );
            }
            format::FrameDecodeResult::UnknownVersion { found, offset } => {
                let path_display = log_path.display();
                anyhow::bail!(
                    "WAL file {path_display} has unknown format version {found} at offset {offset}; \
                     refusing to start"
                );
            }
            format::FrameDecodeResult::Corruption {
                offset,
                stored_crc,
                computed_crc,
            } => {
                let path_display = log_path.display();
                anyhow::bail!(
                    "WAL corruption in {path_display} at offset {offset}: \
                     stored CRC={stored_crc:#010x} computed CRC={computed_crc:#010x}; \
                     refusing to start to avoid replaying corrupt data. \
                     Delete or repair the WAL file to allow startup."
                );
            }
        };

        // Return only entries that have not yet been applied.
        let unapplied: Vec<WalEntry> = entries
            .into_iter()
            .filter(|e| e.sequence > applied_seq)
            .collect();

        Ok(unapplied)
    }
}

impl WalWriter {
    /// Rewrites the partition log retaining only entries above `applied_through`.
    ///
    /// Writes to a `.tmp` file first and atomically renames it over the original
    /// so a crash mid-compaction does not corrupt the WAL.
    async fn compact_log(&self, partition: u32, applied_through: u64) -> anyhow::Result<()> {
        let log_path = self.log_path(partition);
        if !log_path.exists() {
            return Ok(());
        }

        let data = tokio::fs::read(&log_path).await?;
        let all_entries = match format::decode_all(&data) {
            format::FrameDecodeResult::Complete(e) => e,
            format::FrameDecodeResult::CleanEof => Vec::new(),
            format::FrameDecodeResult::TruncatedTail { complete } => complete,
            _ => return Ok(()), // corrupt log — leave it for recovery to handle
        };

        let remaining: Vec<&WalEntry> = all_entries
            .iter()
            .filter(|e| e.sequence > applied_through)
            .collect();

        // Build the compacted log
        let mut compacted = Vec::new();
        for entry in &remaining {
            compacted.extend_from_slice(&format::encode(entry)?);
        }

        let tmp_path = log_path.with_extension("log.tmp");
        tokio::fs::write(&tmp_path, &compacted).await?;
        tokio::fs::rename(&tmp_path, &log_path).await?;

        // Update the in-memory file handle so subsequent appends go to the
        // freshly-rewritten file.
        let new_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;
        let mut guard = self.files.lock().await;
        guard.insert(
            partition,
            PartitionFile {
                file: new_file,
                unfsynced_count: 0,
            },
        );

        Ok(())
    }

    /// Highest sequence this WAL has ever durably observed across **all**
    /// partitions: the max over every `.applied` sidecar watermark AND every
    /// log's max sequence.
    ///
    /// Seeds the live counter to `this + 1` so post-restart writes never reuse a
    /// number at or below a watermark — the recovery filter `e.sequence >
    /// applied_seq` silently drops such writes, so this is the sole guard for
    /// post-restart acked-write durability.
    ///
    /// Must be called **after** `WalRecovery::run` so it observes the watermarks
    /// that recovery's `mark_applied` advanced. Returns `0` for a fresh node
    /// (no partitions / no sequences), so the `+1` seed is `1`.
    ///
    /// # Errors
    ///
    /// Returns an error if a partition log contains mid-file corruption, an
    /// unrecognised magic, or an unknown format version — consistent with
    /// recovery refusing to start on corrupt data.
    pub async fn max_observed_sequence(&self) -> anyhow::Result<u64> {
        let mut global = 0u64;
        for partition in Self::discover_all_partitions(&self.wal_dir)? {
            // The sidecar watermark survives compaction even when the log was
            // rewritten to empty, so it must be read for every partition.
            global = global.max(Self::read_applied_sequence(&self.applied_path(partition)));
            global = global.max(self.max_log_sequence(partition).await?);
        }
        Ok(global)
    }

    /// Discovers partition IDs by scanning `wal_dir` for **both**
    /// `partition-*.log` and `partition-*.applied`, taking the union.
    ///
    /// A compacted-empty partition (log rewritten to empty by `compact_log`)
    /// still carries a live `.applied` watermark, so discovery keyed on `.log`
    /// alone would skip it and the seed would land below that watermark —
    /// reproducing the durability defect. The union defends against that.
    fn discover_all_partitions(wal_dir: &Path) -> anyhow::Result<Vec<u32>> {
        let mut ids: HashSet<u32> = HashSet::new();
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with("partition-") {
                continue;
            }
            let inner = if let Some(rest) = name_str.strip_suffix(".log") {
                rest
            } else if let Some(rest) = name_str.strip_suffix(".applied") {
                rest
            } else {
                continue;
            };
            let inner = &inner["partition-".len()..];
            if let Ok(id) = inner.parse::<u32>() {
                ids.insert(id);
            }
        }
        let mut ids: Vec<u32> = ids.into_iter().collect();
        ids.sort_unstable();
        Ok(ids)
    }

    /// Returns the maximum `WalEntry.sequence` still present in a partition's
    /// log, or `0` if the log is absent/empty.
    ///
    /// Tolerates a truncated tail (uses the intact prefix) using the same
    /// `format::decode_all` classification as `unapplied`; refuses (returns
    /// `Err`) on mid-file corruption, bad magic, or an unknown version.
    async fn max_log_sequence(&self, partition: u32) -> anyhow::Result<u64> {
        let log_path = self.log_path(partition);
        if !log_path.exists() {
            return Ok(0);
        }

        let data = tokio::fs::read(&log_path)
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read WAL file {}: {e}", log_path.display()))?;

        let entries = match format::decode_all(&data) {
            format::FrameDecodeResult::Complete(entries) => entries,
            format::FrameDecodeResult::CleanEof => Vec::new(),
            format::FrameDecodeResult::TruncatedTail { complete } => complete,
            format::FrameDecodeResult::BadMagic { offset } => {
                let path_display = log_path.display();
                anyhow::bail!(
                    "WAL file {path_display} has wrong magic at offset {offset}; \
                     refusing to compute max sequence on corrupt data"
                );
            }
            format::FrameDecodeResult::UnknownVersion { found, offset } => {
                let path_display = log_path.display();
                anyhow::bail!(
                    "WAL file {path_display} has unknown format version {found} at offset {offset}; \
                     refusing to compute max sequence"
                );
            }
            format::FrameDecodeResult::Corruption {
                offset,
                stored_crc,
                computed_crc,
            } => {
                let path_display = log_path.display();
                anyhow::bail!(
                    "WAL corruption in {path_display} at offset {offset}: \
                     stored CRC={stored_crc:#010x} computed CRC={computed_crc:#010x}; \
                     refusing to compute max sequence on corrupt data"
                );
            }
        };

        Ok(entries.iter().map(|e| e.sequence).max().unwrap_or(0))
    }
}

// ===========================================================================
// WalRecovery — startup replay
// ===========================================================================

/// Replays all un-applied WAL entries through an inner `MapDataStore` on startup.
///
/// Called **before** the WebSocket listener accepts connections so clients never
/// observe stale in-flight state. Idempotency is delegated entirely to the inner
/// store's LWW merge: a replayed entry whose HLC timestamp is older than the
/// current stored value is a no-op, so running recovery twice is safe.
pub struct WalRecovery {
    wal: Arc<WalWriter>,
    /// Partition IDs to recover. If empty, all log files in `wal_dir` are used.
    partitions: Vec<u32>,
}

impl WalRecovery {
    /// Creates a `WalRecovery` that will replay entries for the given set of
    /// partition IDs. Pass an empty slice to auto-discover partitions from the
    /// log files in `wal_dir`.
    pub fn new(wal: Arc<WalWriter>, partitions: Vec<u32>) -> Self {
        Self { wal, partitions }
    }

    /// Discovers partition IDs by scanning `wal_dir` for `partition-*.log` files.
    fn discover_partitions(wal_dir: &Path) -> anyhow::Result<Vec<u32>> {
        let mut ids = Vec::new();
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Extract the partition id from the filename pattern partition-NNN.log
            if let Some(rest) = name_str.strip_prefix("partition-") {
                if let Some(inner) = rest.strip_suffix(".log") {
                    if let Ok(id) = inner.parse::<u32>() {
                        ids.push(id);
                    }
                }
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    /// Replays all un-applied entries for every partition through `inner_store`.
    ///
    /// Returns an error (and the caller should exit non-zero) if mid-file
    /// corruption is detected — see `WalWriter::unapplied`. A truncated tail
    /// is tolerated with a WARN log.
    ///
    /// After successful replay the WAL entries are marked applied so a subsequent
    /// clean restart is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if a WAL file contains mid-file CRC corruption, an
    /// unrecognised magic, or an unknown format version.
    pub async fn run(&self, inner_store: Arc<dyn MapDataStore>) -> anyhow::Result<()> {
        let partitions = if self.partitions.is_empty() {
            Self::discover_partitions(&self.wal.wal_dir)?
        } else {
            self.partitions.clone()
        };

        for partition_id in partitions {
            let entries = self.wal.unapplied(partition_id).await?;

            if entries.is_empty() {
                continue;
            }

            tracing::info!(
                partition = partition_id,
                count = entries.len(),
                "WAL recovery: replaying unapplied entries"
            );

            let mut max_seq = 0u64;
            let now = current_millis();

            for entry in &entries {
                // Replay through the inner store so the store's own LWW merge
                // provides idempotency: a stale replay loses the merge.
                let result = match &entry.op {
                    WalOp::Store {
                        value,
                        expiration_time,
                    } => {
                        // Build a RecordValue from the WAL entry.  The timestamp
                        // in the WAL entry is used when present; absent timestamps
                        // (malformed/legacy frames) are treated as always-replay
                        // by using a zero-epoch timestamp.
                        let ts = entry.timestamp.clone().unwrap_or_else(|| Timestamp {
                            millis: 0,
                            counter: 0,
                            node_id: String::new(),
                        });
                        let record_value = RecordValue::Lww {
                            value: value.clone(),
                            timestamp: ts,
                        };
                        inner_store
                            .add(
                                &entry.map,
                                &entry.key,
                                &record_value,
                                expiration_time.unwrap_or(0),
                                now,
                            )
                            .await
                    }
                    WalOp::Remove => inner_store.remove(&entry.map, &entry.key, now).await,
                };

                if let Err(err) = result {
                    tracing::warn!(
                        partition = partition_id,
                        map = %entry.map,
                        key = %entry.key,
                        seq = entry.sequence,
                        error = %err,
                        "WAL recovery: replay failed for entry; skipping"
                    );
                }

                if entry.sequence > max_seq {
                    max_seq = entry.sequence;
                }
            }

            // Mark all replayed entries as applied so a clean restart is a no-op.
            if let Err(err) = self.wal.mark_applied(partition_id, max_seq).await {
                tracing::warn!(
                    partition = partition_id,
                    max_seq = max_seq,
                    error = %err,
                    "WAL recovery: failed to mark entries applied; \
                     next restart will re-replay (safe but redundant)"
                );
            }
        }

        Ok(())
    }
}

/// Current wall-clock time as millis since epoch.
fn current_millis() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use topgun_core::types::Value as TgValue;

    // -----------------------------------------------------------------------
    // Helper types for behavioral tests
    // -----------------------------------------------------------------------

    /// Shared inner-store double that records every replayed operation.
    #[derive(Default)]
    struct ReplayStore {
        adds: tokio::sync::Mutex<Vec<(String, String, crate::storage::record::RecordValue)>>,
        removes: tokio::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl MapDataStore for ReplayStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &crate::storage::record::RecordValue,
            _exp: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.adds
                .lock()
                .await
                .push((map.to_string(), key.to_string(), value.clone()));
            Ok(())
        }

        async fn add_backup(
            &self,
            _map: &str,
            _key: &str,
            _value: &crate::storage::record::RecordValue,
            _exp: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove(&self, map: &str, key: &str, _now: i64) -> anyhow::Result<()> {
            self.removes
                .lock()
                .await
                .push((map.to_string(), key.to_string()));
            Ok(())
        }

        async fn remove_backup(&self, _m: &str, _k: &str, _n: i64) -> anyhow::Result<()> {
            Ok(())
        }

        async fn load(
            &self,
            _map: &str,
            _key: &str,
        ) -> anyhow::Result<Option<crate::storage::record::RecordValue>> {
            Ok(None)
        }

        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, crate::storage::record::RecordValue)>> {
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
            _value: &crate::storage::record::RecordValue,
            _backup: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn reset(&self) {}

        fn is_null(&self) -> bool {
            false
        }
    }

    fn make_wal_entry(seq: u64) -> WalEntry {
        WalEntry {
            map: "m".to_string(),
            key: format!("k{seq}"),
            op: WalOp::Store {
                value: TgValue::String("v".to_string()),
                expiration_time: None,
            },
            timestamp: Some(Timestamp {
                millis: seq,
                counter: 0,
                node_id: "n1".to_string(),
            }),
            sequence: seq,
        }
    }

    // -----------------------------------------------------------------------
    // AC1: append-before-ack — entry durable before append returns
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn wal_writer_append_entry_is_readable_before_ack() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        let entry = make_wal_entry(1);
        // append must complete (durable per policy) before returning
        wal.append(0, &entry).await.unwrap();

        // unapplied must return the entry immediately after append
        let unapplied = wal.unapplied(0).await.unwrap();
        assert_eq!(unapplied.len(), 1, "Entry must be readable after append");
        assert_eq!(unapplied[0].sequence, 1);
    }

    // -----------------------------------------------------------------------
    // AC2: mark_applied removes entry from unapplied set (clean restart is no-op)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn mark_applied_clears_unapplied() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        let entry = make_wal_entry(1);
        wal.append(0, &entry).await.unwrap();

        // Verify it's in the unapplied set before marking
        let before = wal.unapplied(0).await.unwrap();
        assert_eq!(before.len(), 1);

        wal.mark_applied(0, 1).await.unwrap();

        // After marking applied, unapplied must be empty
        let after = wal.unapplied(0).await.unwrap();
        assert!(
            after.is_empty(),
            "After mark_applied, unapplied must be empty (clean restart is no-op)"
        );
    }

    // -----------------------------------------------------------------------
    // AC4a: truncated tail is tolerated — recovery proceeds with intact prefix
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn truncated_tail_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        // Write two complete frames then truncate the second one
        let entry1 = make_wal_entry(1);
        let entry2 = make_wal_entry(2);
        let mut data = format::encode(&entry1).unwrap();
        let frame2 = format::encode(&entry2).unwrap();
        data.extend_from_slice(&frame2[..frame2.len() / 2]);

        let log_path = dir.path().join("partition-000.log");
        tokio::fs::write(&log_path, &data).await.unwrap();

        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
        let unapplied = wal.unapplied(0).await.unwrap();
        // Must recover only the intact first frame, not fail
        assert_eq!(
            unapplied.len(),
            1,
            "Truncated tail must be tolerated; intact prefix must be recovered"
        );
        assert_eq!(unapplied[0].sequence, 1);
    }

    // -----------------------------------------------------------------------
    // AC4b: mid-file checksum mismatch refuses recovery with an error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn mid_file_corruption_refuses_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let entry1 = make_wal_entry(1);
        let entry2 = make_wal_entry(2);

        let mut frame1 = format::encode(&entry1).unwrap();
        let frame2 = format::encode(&entry2).unwrap();
        // Corrupt a byte in frame1's payload (after the 13-byte header)
        frame1[format::FRAME_HEADER_LEN] ^= 0xFF;

        let mut data = frame1;
        data.extend_from_slice(&frame2);

        let log_path = dir.path().join("partition-000.log");
        tokio::fs::write(&log_path, &data).await.unwrap();

        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
        let result = wal.unapplied(0).await;
        assert!(
            result.is_err(),
            "Mid-file CRC corruption must return an error, not silently continue"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.to_lowercase().contains("corrupt") || msg.to_lowercase().contains("crc"),
            "Error message must mention corruption/CRC: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // AC5: idempotent replay — running recovery twice yields identical state
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn recovery_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        let entry = make_wal_entry(10);
        wal.append(0, &entry).await.unwrap();

        let store1 = Arc::new(ReplayStore::default());
        let recovery = WalRecovery::new(Arc::clone(&wal), vec![0]);

        // First recovery run — must succeed.
        recovery
            .run(Arc::clone(&store1) as Arc<dyn MapDataStore>)
            .await
            .unwrap();

        // Counts after first run.
        let first_adds = store1.adds.lock().await.len();

        // Second recovery run on a fresh store — entries are already marked
        // applied so the WAL is compacted; this run should replay nothing.
        let store2 = Arc::new(ReplayStore::default());
        recovery
            .run(Arc::clone(&store2) as Arc<dyn MapDataStore>)
            .await
            .unwrap();
        let second_adds = store2.adds.lock().await.len();

        assert_eq!(
            second_adds, 0,
            "Second recovery run must be a no-op (entries already marked applied). \
             first_adds={first_adds}, second_adds={second_adds}"
        );
    }

    // -----------------------------------------------------------------------
    // WalFsyncPolicy from-str tests
    // -----------------------------------------------------------------------

    #[test]
    fn fsync_policy_parse_per_op() {
        assert_eq!(
            "per_op".parse::<WalFsyncPolicy>().unwrap(),
            WalFsyncPolicy::PerOp
        );
    }

    #[test]
    fn fsync_policy_parse_batched() {
        assert_eq!(
            "batched".parse::<WalFsyncPolicy>().unwrap(),
            WalFsyncPolicy::Batched
        );
    }

    #[test]
    fn fsync_policy_parse_none() {
        assert_eq!(
            "none".parse::<WalFsyncPolicy>().unwrap(),
            WalFsyncPolicy::None
        );
    }

    #[test]
    fn fsync_policy_parse_unknown_is_rejected() {
        // Unknown values must be rejected so a misconfigured deployment surfaces
        // an error at startup rather than silently using the wrong durability level.
        let result = "always".parse::<WalFsyncPolicy>();
        assert!(
            result.is_err(),
            "Unknown policy string should return Err, not a default"
        );
        let err = result.unwrap_err();
        // The error message must name the bad value for operator debuggability.
        assert!(
            err.to_string().contains("always"),
            "Error should name the bad value"
        );
    }

    #[test]
    fn fsync_policy_default_is_batched() {
        assert_eq!(WalFsyncPolicy::default(), WalFsyncPolicy::Batched);
    }

    // -----------------------------------------------------------------------
    // Arc<dyn Wal> object-safety test
    //
    // Demonstrates that `Wal` is object-safe and can be held behind `Arc<dyn
    // Wal>`. This is the injection mechanism used by 307c (production writer)
    // and 307d (sim double).
    // -----------------------------------------------------------------------

    /// Minimal in-memory WAL used to prove object-safety.
    struct InMemoryWal {
        entries: tokio::sync::Mutex<Vec<(u32, WalEntry)>>,
        applied: tokio::sync::Mutex<Vec<(u32, u64)>>,
    }

    impl InMemoryWal {
        fn new() -> Self {
            Self {
                entries: tokio::sync::Mutex::new(Vec::new()),
                applied: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Wal for InMemoryWal {
        async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
            self.entries.lock().await.push((partition, entry.clone()));
            Ok(())
        }

        async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
            self.applied.lock().await.push((partition, sequence));
            Ok(())
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            let guard = self.entries.lock().await;
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

    #[tokio::test]
    async fn wal_trait_is_object_safe() {
        // Constructing an Arc<dyn Wal> verifies the trait is object-safe.
        let wal: Arc<dyn Wal> = Arc::new(InMemoryWal::new());

        let entry = WalEntry {
            map: "map1".to_string(),
            key: "key1".to_string(),
            op: WalOp::Remove,
            timestamp: None,
            sequence: 1,
        };

        wal.append(0, &entry).await.unwrap();

        let unapplied = wal.unapplied(0).await.unwrap();
        assert_eq!(unapplied.len(), 1);

        wal.mark_applied(0, 1).await.unwrap();

        let unapplied_after = wal.unapplied(0).await.unwrap();
        assert!(
            unapplied_after.is_empty(),
            "No entries should remain after mark_applied"
        );
    }
}
