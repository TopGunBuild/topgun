//! Write-Ahead Log (WAL) trait surface, concrete writer, and recovery.
//!
//! Defines the frozen contract and implements the production file-backed
//! writer (`WalWriter`) and startup recovery (`WalRecovery`).
//!
//! The WAL is per-partition: one append-only file per partition under a
//! configured `wal_dir`. `WalWriter` implements the `Wal` trait; `WalRecovery`
//! replays un-applied entries through the inner store on startup.

pub mod format;
pub mod segment;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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

use crate::storage::wal::segment::Segment;

/// All segments of one partition, with the active segment and the sealed set
/// guarded by **separate** locks.
///
/// Split-locking is the structural guard for the lock-discipline invariant:
/// `append` takes only `active`, GC takes only `sealed`, so deletion of a sealed
/// segment never blocks an in-flight append to the active segment. Rotation is
/// the only operation that briefly holds both (active for the fsync-old →
/// create-new → fsync-dir → swap, sealed only for the push of the frozen segment).
struct PartitionHandle {
    /// The append-target segment. Appends mutate only this, under this lock.
    active: AsyncMutex<Segment>,
    /// Sealed, immutable segments in ascending `first_seq` order (oldest at the
    /// front). GC reclaims a watermark-covered prefix from the front.
    sealed: AsyncMutex<Vec<Segment>>,
}

/// Production append-only WAL writer.
///
/// Each partition's log is split into a sequence of segment files named
/// `partition-{id:03}-{first_seq:020}.log` (see `segment`): appends always target
/// the single active segment, and `mark_applied` seals the active segment, rotates
/// a fresh one in, then GC-deletes sealed segments fully covered by the applied
/// watermark. Applied-sequence tracking uses a sidecar file
/// `partition-{id:03}.applied` that stores the highest applied sequence number as
/// a big-endian u64 so recovery can skip already-durable entries — and so GC and
/// the post-restart sequence seed have a crash-durable watermark.
///
/// Thread safety: each partition is guarded by its own `PartitionHandle` (split
/// active/sealed locks) so concurrent appends to different partitions do not block
/// each other and GC never serializes against appends.
pub struct WalWriter {
    wal_dir: PathBuf,
    policy: WalFsyncPolicy,
    /// Per-partition segment handles, lazily opened on first append/discovery.
    partitions: AsyncMutex<HashMap<u32, Arc<PartitionHandle>>>,
    /// Batched-commit group-commit timer: shared notifier woken every ~10 ms.
    /// When `policy == Batched`, the writer also fsyncs after every 100 ops.
    batch_flush_tx: tokio::sync::watch::Sender<()>,
}

/// fsyncs a directory so a just-created (or just-renamed/unlinked) entry within it
/// is durable. On most filesystems the parent-dir fsync is what makes a new file's
/// directory entry survive a crash; without it, a freshly-created segment that has
/// been written and acked can vanish on power loss.
fn fsync_dir(dir: &Path) -> anyhow::Result<()> {
    let handle = std::fs::File::open(dir)
        .map_err(|e| anyhow::anyhow!("Cannot open WAL dir {} for fsync: {e}", dir.display()))?;
    handle
        .sync_all()
        .map_err(|e| anyhow::anyhow!("Cannot fsync WAL dir {}: {e}", dir.display()))?;
    Ok(())
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
            partitions: AsyncMutex::new(HashMap::new()),
            batch_flush_tx,
        });

        // For the Batched policy, spawn a background task that fsyncs every open
        // active segment every ~10 ms so group-commit amortises fsync cost. Only
        // the active segment is ever appended to, so only it can have un-fsynced
        // frames; sealed segments were already fsynced at seal time.
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
                    // Snapshot the handle list under the map lock, then fsync each
                    // active segment under its own lock so the timer never blocks
                    // appends to other partitions.
                    let handles: Vec<Arc<PartitionHandle>> =
                        { w.partitions.lock().await.values().cloned().collect() };
                    for handle in handles {
                        let mut active = handle.active.lock().await;
                        if active.unfsynced_count() > 0 {
                            // Surface batch-fsync failures: swallowing the error
                            // would leave acked frames non-durable under Batched
                            // with no operator signal that durability is
                            // compromised.
                            if let Err(e) = active.sync_data().await {
                                tracing::error!(
                                    segment = ?active.path(),
                                    error = %e,
                                    "WAL batch fsync failed; acked frames may not be durable"
                                );
                            }
                        }
                    }
                }
            });
        }

        Ok(writer)
    }

    /// Path of the active segment for a fresh partition (seeded at `first_seq`).
    fn first_segment_path(&self, partition: u32) -> PathBuf {
        self.wal_dir
            .join(segment::format_segment_filename(partition, 0))
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

    /// Writes the highest applied sequence to the sidecar file atomically and
    /// crash-durably (write `.tmp`, fsync it, rename, fsync the parent dir).
    ///
    /// The rename is atomic but not crash-durable on its own. This watermark is now
    /// load-bearing twice over: it is the GC gate (sealed segments at or below it
    /// are unlinked) and the restart seed for `max_observed_sequence`. If it were
    /// lost on a crash, a fully-compacted partition would under-seed the sequence
    /// counter and reuse a number the recovery filter then silently drops. fsyncing
    /// the data and the directory entry closes that hole.
    fn write_applied_sequence(path: &Path, seq: u64) -> anyhow::Result<()> {
        use std::io::Write as _;
        let tmp = path.with_extension("applied.tmp");
        {
            let mut file = std::fs::File::create(&tmp)
                .map_err(|e| anyhow::anyhow!("Cannot create applied tmp {}: {e}", tmp.display()))?;
            file.write_all(&seq.to_be_bytes())
                .map_err(|e| anyhow::anyhow!("Cannot write applied tmp {}: {e}", tmp.display()))?;
            file.sync_all()
                .map_err(|e| anyhow::anyhow!("Cannot fsync applied tmp {}: {e}", tmp.display()))?;
        }
        std::fs::rename(&tmp, path)?;
        if let Some(parent) = path.parent() {
            fsync_dir(parent)?;
        }
        Ok(())
    }

    /// Discovers a single partition's segment files on disk, returning their
    /// `(first_seq, path)` pairs sorted ascending by `first_seq`.
    ///
    /// Recovery and lazy handle-open both rely on the filename convention to order
    /// segments without opening them.
    fn discover_partition_segments(
        wal_dir: &Path,
        partition: u32,
    ) -> anyhow::Result<Vec<(u64, PathBuf)>> {
        let mut found: Vec<(u64, PathBuf)> = Vec::new();
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some((p, first_seq)) = segment::parse_segment_filename(&name_str) {
                if p == partition {
                    found.push((first_seq, entry.path()));
                }
            }
        }
        found.sort_by_key(|(first_seq, _)| *first_seq);
        Ok(found)
    }

    /// Returns the live `PartitionHandle`, opening it from disk on first use.
    ///
    /// On open, the partition's existing segment files are discovered by filename
    /// (ascending `first_seq`). The highest-`first_seq` segment becomes the active
    /// append target (reopened in append mode, its `max_seq` decoded from its
    /// frames); the rest are recorded as sealed with their decoded `max_seq`. A
    /// partition with no segment files starts with a single empty active segment at
    /// `first_seq = 0`.
    async fn handle(&self, partition: u32) -> anyhow::Result<Arc<PartitionHandle>> {
        {
            let map = self.partitions.lock().await;
            if let Some(h) = map.get(&partition) {
                return Ok(Arc::clone(h));
            }
        }

        let mut segments = Self::discover_partition_segments(&self.wal_dir, partition)?;

        let mut sealed: Vec<Segment> = Vec::new();
        let active: Segment = if let Some((active_first_seq, active_path)) = segments.pop() {
            // The highest-`first_seq` segment is the active append target; the
            // remainder are sealed (decoded for their authoritative `max_seq`).
            for (first_seq, path) in segments {
                let max_seq = Self::decode_segment_max_seq(&path).await?;
                sealed.push(Segment::sealed_existing(path, first_seq, max_seq));
            }
            // Decode the active segment's intact prefix (tolerating a torn tail
            // from a crash mid-append; mid-file corruption stays fatal). If the
            // intact prefix is shorter than the file on disk, the tail was torn:
            // truncate it durably BEFORE opening the append handle. Left in place,
            // the torn bytes would sit between the intact prefix and the new
            // appends; once this segment is later sealed (becomes non-last), the
            // torn region becomes fatal mid-file corruption that refuses recovery.
            let data = tokio::fs::read(&active_path).await.map_err(|e| {
                anyhow::anyhow!("Cannot read WAL segment {}: {e}", active_path.display())
            })?;
            let entries = Self::decode_segment_or_refuse(&active_path, &data, true)?;
            // Exact byte length of the intact prefix: re-encode each intact entry
            // (the codec is deterministic) and sum the frame lengths.
            let mut intact_len: u64 = 0;
            for e in &entries {
                intact_len = intact_len.saturating_add(format::encode(e)?.len() as u64);
            }
            if intact_len < data.len() as u64 {
                tracing::warn!(
                    path = ?active_path,
                    file_len = data.len(),
                    intact_len,
                    "WAL active segment has a torn tail; truncating to intact prefix \
                     before reopening for append"
                );
                let truncate_file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(&active_path)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot open WAL segment {} for truncation: {e}",
                            active_path.display()
                        )
                    })?;
                truncate_file.set_len(intact_len).await.map_err(|e| {
                    anyhow::anyhow!(
                        "Cannot truncate torn WAL segment {}: {e}",
                        active_path.display()
                    )
                })?;
                truncate_file.sync_all().await.map_err(|e| {
                    anyhow::anyhow!(
                        "Cannot fsync truncated WAL segment {}: {e}",
                        active_path.display()
                    )
                })?;
            }
            let max_seq = entries.iter().map(|e| e.sequence).max().unwrap_or(0);
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&active_path)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Cannot open WAL segment {}: {e}", active_path.display())
                })?;
            let mut seg = Segment::new_active(active_path, active_first_seq, file);
            seg.set_recovered_max_seq(max_seq);
            seg
        } else {
            let path = self.first_segment_path(partition);
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|e| anyhow::anyhow!("Cannot open WAL segment {}: {e}", path.display()))?;
            // A freshly-created segment's dir entry must be durable before any
            // append to it is acked.
            fsync_dir(&self.wal_dir)?;
            Segment::new_active(path, 0, file)
        };

        let mut map = self.partitions.lock().await;
        // Another task may have opened the handle while we were doing I/O without
        // the map lock; honour the first one installed.
        if let Some(h) = map.get(&partition) {
            return Ok(Arc::clone(h));
        }
        let handle = Arc::new(PartitionHandle {
            active: AsyncMutex::new(active),
            sealed: AsyncMutex::new(sealed),
        });
        map.insert(partition, Arc::clone(&handle));
        Ok(handle)
    }

    /// Decodes the highest sequence number present in a segment file, tolerating a
    /// truncated tail on the file (returns the intact prefix's max) and refusing on
    /// mid-file corruption. Returns the file's `first_seq` floor when it is empty —
    /// the caller distinguishes empty from a genuine single-frame segment via the
    /// frame count, not this value.
    async fn decode_segment_max_seq(path: &Path) -> anyhow::Result<u64> {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read WAL segment {}: {e}", path.display()))?;
        let entries = Self::decode_segment_or_refuse(path, &data, true)?;
        Ok(entries.iter().map(|e| e.sequence).max().unwrap_or(0))
    }

    /// Decodes every frame in one segment's bytes, applying the shared corruption
    /// taxonomy. `tolerate_truncated_tail` controls whether a truncated tail is a
    /// recoverable WARN (active / last segment) or a fatal refusal (a sealed,
    /// non-last segment must never be torn).
    fn decode_segment_or_refuse(
        path: &Path,
        data: &[u8],
        tolerate_truncated_tail: bool,
    ) -> anyhow::Result<Vec<WalEntry>> {
        match format::decode_all(data) {
            format::FrameDecodeResult::Complete(entries) => Ok(entries),
            format::FrameDecodeResult::CleanEof => Ok(Vec::new()),
            format::FrameDecodeResult::TruncatedTail { complete } => {
                if tolerate_truncated_tail {
                    tracing::warn!(
                        path = ?path,
                        "WAL tail truncated (crash mid-write); replaying intact prefix"
                    );
                    Ok(complete)
                } else {
                    let path_display = path.display();
                    anyhow::bail!(
                        "WAL segment {path_display} has a truncated tail but is not the active \
                         segment; a torn non-last segment is corruption — refusing to start"
                    );
                }
            }
            format::FrameDecodeResult::BadMagic { offset } => {
                let path_display = path.display();
                anyhow::bail!(
                    "WAL file {path_display} has wrong magic at offset {offset}; \
                     refusing to start to avoid replaying corrupt data"
                );
            }
            format::FrameDecodeResult::UnknownVersion { found, offset } => {
                let path_display = path.display();
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
                let path_display = path.display();
                anyhow::bail!(
                    "WAL corruption in {path_display} at offset {offset}: \
                     stored CRC={stored_crc:#010x} computed CRC={computed_crc:#010x}; \
                     refusing to start to avoid replaying corrupt data. \
                     Delete or repair the WAL file to allow startup."
                );
            }
        }
    }
}

#[async_trait]
impl Wal for WalWriter {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        let frame = format::encode(entry)?;

        let handle = self.handle(partition).await?;
        // Take ONLY the active-append lock. GC of sealed segments runs under a
        // distinct lock and never blocks this path; rotation briefly takes this
        // same lock, which is correct — an append and a rotate on the same
        // partition must serialize so a seal never freezes a half-written frame.
        let mut active = handle.active.lock().await;

        let count = active.append_frame(&frame, entry.sequence).await?;

        match self.policy {
            WalFsyncPolicy::PerOp => {
                active.sync_data().await?;
            }
            WalFsyncPolicy::Batched => {
                // Flush immediately when the batch threshold is reached so
                // high-throughput bursts don't wait for the timer task.
                if count >= 100 {
                    active.sync_data().await?;
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
        let applied_path = self.applied_path(partition);
        // Advance the watermark monotonically; never regress it.
        let current = Self::read_applied_sequence(&applied_path);
        let watermark = sequence.max(current);
        if watermark > current {
            // Durably advance the watermark BEFORE any sealed segment is unlinked:
            // a crash after an unlink but before the watermark fsync would
            // under-seed max_observed_sequence on restart and reuse a sequence the
            // recovery filter then silently drops. The fsync (data + parent dir)
            // closes that hole.
            Self::write_applied_sequence(&applied_path, watermark).map_err(|e| {
                anyhow::anyhow!(
                    "Cannot write applied sidecar {}: {e}",
                    applied_path.display()
                )
            })?;
        }

        let handle = self.handle(partition).await?;

        // --- Seal + rotate (RULE-ordered, under the active-append lock) ---
        // Only rotate when the active segment actually holds frames. Sealing an
        // empty active segment would leave a zero-length sealed file the next
        // append could never reach and GC would have to special-case.
        {
            let mut active = handle.active.lock().await;
            if active.has_frames() {
                // Durably fsync the current active's data FIRST, before any swap or
                // new-file creation. A sealed segment is non-last in the live
                // ordering, and a torn tail on a non-last segment is fatal during
                // recovery — so its data must be synced before it is frozen. Doing
                // the only fallible fsync here (while the old active is still
                // installed and no new file exists yet) means a failure leaves no
                // orphan: returning early keeps the old active in place with no
                // dangling new segment.
                active.sync_data().await?;

                let next_first_seq = active.max_seq().saturating_add(1);
                let new_path = self
                    .wal_dir
                    .join(segment::format_segment_filename(partition, next_first_seq));
                let new_file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&new_path)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Cannot open WAL segment {}: {e}", new_path.display())
                    })?;
                // (c) fsync the parent dir so the new active file's dir entry is
                // durable BEFORE any append is acked to it (before this lock is
                // released).
                fsync_dir(&self.wal_dir)?;

                // Swap in the fresh active, take the old one out to freeze it. The
                // old segment's data was already fsynced above, so the freeze is
                // the infallible `into_sealed` — no fallible fsync runs AFTER the
                // swap, which guarantees the old active can never be dropped
                // (orphaned) by a failing fsync after it has been replaced.
                let new_active = Segment::new_active(new_path, next_first_seq, new_file);
                let old_active = std::mem::replace(&mut *active, new_active);
                let frozen = old_active.into_sealed();
                // The sealed-set lock is taken only for the push, never held across
                // the append-lock release, so appenders are not serialized by GC.
                handle.sealed.lock().await.push(frozen);
                // Active-append lock released here.
            }
        }

        // --- GC sealed segments fully covered by the watermark ---
        // Takes ONLY the sealed-set lock; never the active-append lock. Appends in
        // flight to the active segment are not blocked by this deletion.
        let reclaimed: Vec<PathBuf> = {
            let mut sealed = handle.sealed.lock().await;
            // Reclaimable sealed segments are a prefix of the ascending-ordered set
            // (oldest first), valid because sequences are monotonic and gap-free.
            let reclaim = sealed
                .iter()
                .take_while(|s| s.max_seq() <= watermark)
                .count();
            sealed
                .drain(..reclaim)
                .map(|s| s.path().to_path_buf())
                .collect()
        };

        for path in &reclaimed {
            tokio::fs::remove_file(path).await.map_err(|e| {
                anyhow::anyhow!("Cannot unlink sealed WAL segment {}: {e}", path.display())
            })?;
        }
        // A single parent-dir fsync after the unlink batch makes the reclamation
        // durable. Per-unlink dir fsync is unnecessary: GC is resumable and replay
        // is idempotent under the watermark filter, so a crash between unlinks is
        // safe.
        if !reclaimed.is_empty() {
            fsync_dir(&self.wal_dir)?;
        }

        Ok(())
    }

    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        let applied_seq = Self::read_applied_sequence(&self.applied_path(partition));
        let segments = Self::discover_partition_segments(&self.wal_dir, partition)?;
        if segments.is_empty() {
            return Ok(Vec::new());
        }

        let last_idx = segments.len() - 1;
        let mut all: Vec<WalEntry> = Vec::new();
        for (idx, (_first_seq, path)) in segments.into_iter().enumerate() {
            let data = match tokio::fs::read(&path).await {
                Ok(data) => data,
                // A concurrent GC can unlink a sealed segment between the directory
                // scan above and this read. A GC'd segment is `<= watermark` by
                // construction, so its frames are already applied and the later
                // `retain(|e| e.sequence > applied_seq)` would drop them anyway —
                // skipping the vanished file is correct, not a lost frame.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Cannot read WAL segment {}: {e}",
                        path.display()
                    ));
                }
            };
            // A truncated tail is tolerated ONLY on the active (last) segment; a
            // torn tail on a sealed, non-last segment is corruption.
            let tolerate_tail = idx == last_idx;
            let entries = Self::decode_segment_or_refuse(&path, &data, tolerate_tail)?;
            all.extend(entries);
        }

        // Frames are written in ascending sequence order within and across
        // segments, but sort defensively so the ordering contract holds even if a
        // future path interleaves.
        all.sort_by_key(|e| e.sequence);
        all.retain(|e| e.sequence > applied_seq);
        Ok(all)
    }
}

impl WalWriter {
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
            // The sidecar watermark survives GC even when every segment was
            // reclaimed (compacted-to-empty partition), so it must be read for
            // every partition.
            global = global.max(Self::read_applied_sequence(&self.applied_path(partition)));
            global = global.max(self.max_log_sequence(partition).await?);
        }
        Ok(global)
    }

    /// Discovers partition IDs by scanning `wal_dir` for **both** segment files
    /// (`partition-NNN-SSS.log`) and `.applied` sidecars, taking the union.
    ///
    /// A compacted-to-empty partition (all segments GC'd) still carries a live
    /// `.applied` watermark, so discovery keyed on segment files alone would skip
    /// it and the seed would land below that watermark — reproducing the durability
    /// defect. The union defends against that.
    fn discover_all_partitions(wal_dir: &Path) -> anyhow::Result<Vec<u32>> {
        let mut ids: HashSet<u32> = HashSet::new();
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some((partition, _first_seq)) = segment::parse_segment_filename(&name_str) {
                ids.insert(partition);
                continue;
            }
            if let Some(rest) = name_str.strip_prefix("partition-") {
                if let Some(inner) = rest.strip_suffix(".applied") {
                    if let Ok(id) = inner.parse::<u32>() {
                        ids.insert(id);
                    }
                }
            }
        }
        let mut ids: Vec<u32> = ids.into_iter().collect();
        ids.sort_unstable();
        Ok(ids)
    }

    /// Returns the maximum `WalEntry.sequence` still present across **all** of a
    /// partition's segment files, or `0` if it has none.
    ///
    /// Tolerates a truncated tail (uses the intact prefix) ONLY on the active
    /// (last) segment, and refuses on mid-file corruption / bad magic / unknown
    /// version in any segment — the same taxonomy `unapplied` uses.
    async fn max_log_sequence(&self, partition: u32) -> anyhow::Result<u64> {
        let segments = Self::discover_partition_segments(&self.wal_dir, partition)?;
        if segments.is_empty() {
            return Ok(0);
        }
        let last_idx = segments.len() - 1;
        let mut global = 0u64;
        for (idx, (_first_seq, path)) in segments.into_iter().enumerate() {
            let data = tokio::fs::read(&path)
                .await
                .map_err(|e| anyhow::anyhow!("Cannot read WAL segment {}: {e}", path.display()))?;
            let entries = Self::decode_segment_or_refuse(&path, &data, idx == last_idx)?;
            if let Some(m) = entries.iter().map(|e| e.sequence).max() {
                global = global.max(m);
            }
        }
        Ok(global)
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

    /// Discovers partition IDs by scanning `wal_dir` for segment files
    /// (`partition-NNN-SSS.log`).
    fn discover_partitions(wal_dir: &Path) -> anyhow::Result<Vec<u32>> {
        let mut ids: HashSet<u32> = HashSet::new();
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some((partition, _first_seq)) = segment::parse_segment_filename(&name_str) {
                ids.insert(partition);
            }
        }
        let mut ids: Vec<u32> = ids.into_iter().collect();
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

        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn crate::storage::map_data_store::LeafSink,
        ) -> anyhow::Result<()> {
            // WAL replay test store records calls only; no durable leaves.
            Ok(())
        }

        async fn scan_values(
            &self,
            _map: &str,
            _is_backup: bool,
            _max_batch_cost: u64,
        ) -> anyhow::Result<crate::storage::map_data_store::ScanBatch> {
            Ok(crate::storage::map_data_store::ScanBatch::default())
        }

        async fn scan_values_batched(
            &self,
            _map: &str,
            _is_backup: bool,
            _cursor: crate::storage::map_data_store::ScanCursor,
            _max_batch_cost: u64,
        ) -> anyhow::Result<crate::storage::map_data_store::ScanBatch> {
            Ok(crate::storage::map_data_store::ScanBatch::default())
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
    // Concurrency regression: seal/rotate must not drop appends that interleave
    // with it. mark_applied(0) seals + rotates the active segment while an
    // appender races on the same partition. The watermark stays 0 so GC reclaims
    // nothing and every appended seq must survive in unapplied — proving the
    // RULE: no acked frame is lost when a rotation lands between two appends.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn compaction_does_not_drop_appends_racing_on_same_partition() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();
        let total: u64 = 2000;

        let appender = {
            let wal = Arc::clone(&wal);
            tokio::spawn(async move {
                for seq in 1..=total {
                    wal.append(0, &make_wal_entry(seq)).await.unwrap();
                }
            })
        };
        let compactor = {
            let wal = Arc::clone(&wal);
            tokio::spawn(async move {
                // applied_through = 0 → compaction retains every entry but still
                // performs the read→rewrite→rename that races the appender.
                for _ in 0..total {
                    wal.mark_applied(0, 0).await.unwrap();
                    tokio::task::yield_now().await;
                }
            })
        };

        appender.await.unwrap();
        compactor.await.unwrap();

        let unapplied = wal.unapplied(0).await.unwrap();
        let seen: HashSet<u64> = unapplied.iter().map(|e| e.sequence).collect();
        let missing: Vec<u64> = (1..=total).filter(|s| !seen.contains(s)).collect();
        assert!(
            missing.is_empty(),
            "compaction dropped {} acked appends (first few: {:?})",
            missing.len(),
            &missing[..missing.len().min(10)]
        );
    }

    // -----------------------------------------------------------------------
    // RULE stress regression (promoted from the G9 depth-audit scratch suite):
    // no acked WAL frame may be dropped by seal/rotate/GC under concurrent
    // same-partition append. Two variants:
    //
    //   Variant A — watermark pinned at 0: GC reclaims nothing, so EVERY acked
    //     sequence must survive in `unapplied`. This isolates the seal/rotate
    //     interleave from GC (a rotation landing between two appends must not
    //     orphan an acked frame).
    //
    //   Variant B — a non-zero, advancing watermark that lags the appender: this
    //     actually exercises seal → rotate → unlink. The invariant is the RULE:
    //     every acked sequence `s` must be either present in `unapplied` (it is
    //     above the final watermark) OR provably applied (`s <= final watermark`,
    //     so a correct `unapplied` filter excludes it). Zero acked sequences may
    //     be silently dropped — i.e. absent from `unapplied` while also above the
    //     watermark.
    // -----------------------------------------------------------------------

    /// Variant A — pinned watermark 0: every acked sequence survives in
    /// `unapplied` because GC reclaims nothing while seal/rotate keeps racing the
    /// appender. This is the segment-rotation analogue of the audit's original
    /// `stress_compact_races_append_loses_acked_writes` repro.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn stress_compact_races_append_loses_acked_writes() {
        const N: u64 = 2000;

        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        let appender = {
            let wal = Arc::clone(&wal);
            tokio::spawn(async move {
                let mut acked = HashSet::new();
                for seq in 1..=N {
                    wal.append(0, &make_wal_entry(seq)).await.unwrap();
                    acked.insert(seq);
                }
                acked
            })
        };

        let compactor = {
            let wal = Arc::clone(&wal);
            tokio::spawn(async move {
                // watermark == 0 ⇒ GC retains every entry, but seal/rotate still
                // freezes the active segment under the appender's feet on every
                // call — the window the RULE forbids from dropping a frame.
                for _ in 0..4000 {
                    wal.mark_applied(0, 0).await.unwrap();
                    tokio::task::yield_now().await;
                }
            })
        };

        let acked = appender.await.unwrap();
        compactor.await.unwrap();

        // Watermark is still 0, so `unapplied` must return every acked frame.
        let present: HashSet<u64> = wal
            .unapplied(0)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.sequence)
            .collect();

        let lost: Vec<u64> = acked.difference(&present).copied().collect();
        assert!(
            lost.is_empty(),
            "{} acked sequences silently dropped by seal/rotate race (e.g. {:?})",
            lost.len(),
            &lost.iter().take(10).collect::<Vec<_>>()
        );
    }

    /// Variant B — an advancing, appender-lagging watermark drives real
    /// seal/rotate/GC (sealed segments below the watermark are unlinked). The
    /// RULE: no acked sequence may be both absent from `unapplied` and above the
    /// final watermark.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn stress_advancing_watermark_drops_no_acked_write() {
        use std::sync::atomic::{AtomicU64, Ordering};

        const N: u64 = 4000;
        // The GC task advances the watermark to this many sequences behind the
        // appender's latest ack, so a band of recent sequences always stays above
        // the watermark (in live segments) while older ones are genuinely GC'd.
        const LAG: u64 = 64;

        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        // Highest sequence the appender has acked so far; the GC task reads it to
        // pick a watermark that lags behind the live frontier.
        let acked_frontier = Arc::new(AtomicU64::new(0));

        let appender = {
            let wal = Arc::clone(&wal);
            let frontier = Arc::clone(&acked_frontier);
            tokio::spawn(async move {
                for seq in 1..=N {
                    wal.append(0, &make_wal_entry(seq)).await.unwrap();
                    // Publish the ack only AFTER append returns Ok, so the GC task
                    // can never advance the watermark past a not-yet-acked frame.
                    frontier.store(seq, Ordering::Release);
                }
            })
        };

        let compactor = {
            let wal = Arc::clone(&wal);
            let frontier = Arc::clone(&acked_frontier);
            tokio::spawn(async move {
                loop {
                    let acked = frontier.load(Ordering::Acquire);
                    // Lag the watermark behind the live frontier so seal/rotate +
                    // unlink run continuously without the watermark ever reaching
                    // an unacked sequence.
                    let watermark = acked.saturating_sub(LAG);
                    wal.mark_applied(0, watermark).await.unwrap();
                    tokio::task::yield_now().await;
                    if acked >= N {
                        break;
                    }
                }
            })
        };

        appender.await.unwrap();
        compactor.await.unwrap();

        // Final watermark observed on disk: every acked sequence at or below it is
        // provably applied and a correct `unapplied` filter excludes it.
        let final_watermark = WalWriter::read_applied_sequence(&wal.applied_path(0));
        let present: HashSet<u64> = wal
            .unapplied(0)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.sequence)
            .collect();

        // The RULE: every acked sequence is either still live (in `unapplied`) or
        // below the watermark (provably applied). A sequence that is BOTH absent
        // from `unapplied` AND above the watermark was silently dropped.
        let dropped: Vec<u64> = (1..=N)
            .filter(|s| !present.contains(s) && *s > final_watermark)
            .collect();
        assert!(
            dropped.is_empty(),
            "{} acked sequences dropped above the final watermark {} (e.g. {:?})",
            dropped.len(),
            final_watermark,
            &dropped[..dropped.len().min(10)]
        );
    }

    // -----------------------------------------------------------------------
    // AC3 — lock discipline (structural): GC of sealed segments must NEVER block
    // an in-flight append to the active segment. The two paths take DISTINCT
    // locks (`PartitionHandle::active` vs `PartitionHandle::sealed`), so GC can
    // run to completion while the active-append lock is held by someone else.
    //
    // We assert this by reproducing `mark_applied`'s exact GC body (drain the
    // watermark-covered prefix of the sealed set + unlink) while HOLDING the
    // active-append lock the whole time. If GC took the active lock, this would
    // deadlock; the test completing proves the locks are separate and that an
    // append in flight (which would hold `active`) does not serialize against GC.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn gc_does_not_block_inflight_append() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        // Build a partition with several sealed segments below a watermark by
        // appending and sealing repeatedly. Each mark_applied seals the active
        // segment and rotates a fresh one; watermark 0 keeps them sealed (not yet
        // GC'd) so the sealed set is populated for the GC-under-lock assertion.
        for seq in 1..=8u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
            wal.mark_applied(0, 0).await.unwrap();
        }

        let handle = wal.handle(0).await.unwrap();

        // Hold the active-append lock for the entire GC run. An in-flight append
        // holds exactly this lock; if GC needed it, this would deadlock.
        let active_guard = handle.active.lock().await;

        // Reproduce mark_applied's GC body verbatim: it takes ONLY the sealed-set
        // lock, drains the watermark-covered prefix, and unlinks — never touching
        // the active lock we are holding above.
        let watermark = u64::MAX; // reclaim every sealed segment
        let reclaimed: Vec<PathBuf> = {
            let mut sealed = handle.sealed.lock().await;
            let reclaim = sealed
                .iter()
                .take_while(|s| s.max_seq() <= watermark)
                .count();
            assert!(
                reclaim > 0,
                "test setup must leave at least one sealed segment to GC"
            );
            sealed
                .drain(..reclaim)
                .map(|s| s.path().to_path_buf())
                .collect()
        };
        for path in &reclaimed {
            tokio::fs::remove_file(path).await.unwrap();
        }

        // Reaching here while still holding the active lock proves GC made full
        // progress without the active-append lock — the AC3 lock-discipline
        // guarantee. Drop the guard explicitly to document the held duration.
        drop(active_guard);

        // Sanity: the sealed set is now empty (everything GC'd) and the active
        // segment is untouched and still appendable.
        assert!(handle.sealed.lock().await.is_empty());
        wal.append(0, &make_wal_entry(9)).await.unwrap();
    }

    // -----------------------------------------------------------------------
    // HIGH-1 regression: a crash-torn tail on the ACTIVE segment must be
    // truncated on reopen, BEFORE further appends and BEFORE the segment is later
    // sealed. Without truncation the torn bytes stay between the pre-crash prefix
    // and the post-reopen appends; once the segment seals (becomes non-last) the
    // torn region is fatal mid-file corruption and recovery refuses to start,
    // losing every frame written after the tear.
    //
    // Repro: append frames to the active segment, simulate a torn tail by
    // appending garbage bytes to the segment file on disk, reopen the writer,
    // append MORE frames, then mark_applied(0) to seal+rotate the (truncated-then-
    // extended) segment. Reopen once more and assert recovery is Ok and every
    // acked frame before AND after the tear is recovered (the torn partial frame
    // may be absent). Fails without Fix 1 (seal hits mid-file corruption → Err);
    // passes with it.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn torn_active_tail_truncated_on_reopen_survives_seal() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        // Phase 1: write frames 1..=3 to the active segment, then drop the writer.
        {
            let wal = WalWriter::new(dir_path.clone(), WalFsyncPolicy::None).unwrap();
            for seq in 1..=3u64 {
                wal.append(0, &make_wal_entry(seq)).await.unwrap();
            }
        }

        // Simulate a crash mid-append: append a few garbage / partial bytes to the
        // active segment file so its tail is torn (not a clean frame boundary).
        let active_path = dir_path.join(segment::format_segment_filename(0, 0));
        {
            use tokio::io::AsyncWriteExt as _;
            let mut f = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&active_path)
                .await
                .unwrap();
            // Start a frame (valid magic+version) but truncate it — a classic torn
            // mid-append tail.
            f.write_all(&format::FRAME_MAGIC.to_be_bytes())
                .await
                .unwrap();
            f.write_all(&[format::FRAME_VERSION, 0xAB, 0xCD])
                .await
                .unwrap();
            f.flush().await.unwrap();
        }

        // Phase 2: reopen the writer (Fix 1 truncates the torn tail on handle open),
        // append frames 4..=6, then seal+rotate via mark_applied. Watermark 0 keeps
        // nothing GC'd so every frame must remain recoverable.
        {
            let wal = WalWriter::new(dir_path.clone(), WalFsyncPolicy::None).unwrap();
            for seq in 4..=6u64 {
                wal.append(0, &make_wal_entry(seq)).await.unwrap();
            }
            // Seals the (now-truncated-then-extended) segment and rotates a fresh
            // one. Without Fix 1 the sealed segment carries the torn region as a
            // mid-file gap.
            wal.mark_applied(0, 0).await.unwrap();
        }

        // Phase 3: fresh writer over the same dir — recovery must succeed and
        // return every acked frame (1..=6). Without Fix 1, the sealed segment's
        // torn region is fatal mid-file corruption → Err here.
        let wal = WalWriter::new(dir_path.clone(), WalFsyncPolicy::None).unwrap();
        let unapplied = wal
            .unapplied(0)
            .await
            .expect("recovery must succeed: a sealed segment must not carry a torn tail");
        let seen: HashSet<u64> = unapplied.iter().map(|e| e.sequence).collect();
        let missing: Vec<u64> = (1..=6u64).filter(|s| !seen.contains(s)).collect();
        assert!(
            missing.is_empty(),
            "acked frames lost across the torn-tail truncate+seal: {missing:?} (seen: {seen:?})"
        );
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

        let log_path = dir.path().join(segment::format_segment_filename(0, 1));
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

        let log_path = dir.path().join(segment::format_segment_filename(0, 1));
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
