//! Write-Ahead Log (WAL) trait surface, concrete writer, and recovery.
//!
//! Defines the frozen contract and implements the production file-backed
//! writer (`WalWriter`) and startup recovery (`WalRecovery`).
//!
//! The WAL is per-partition: one append-only file per partition under a
//! configured `wal_dir`. `WalWriter` implements the `Wal` trait; `WalRecovery`
//! replays un-applied entries through the inner store on startup.
//!
//! A WAL is opened over a store (`WalWriter::open_over_store`) that keeps the
//! applied watermark, committed with the data it covers. The `.applied` files
//! are only the rollback hints a clean stop writes for an older binary.

pub mod format;
pub mod segment;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_core::PARTITION_COUNT;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::{OrMapEntry, RecordValue};
use crate::storage::sealing::Keyring;

// ---------------------------------------------------------------------------
// WalFsyncPolicy
// ---------------------------------------------------------------------------

/// Controls how aggressively the WAL writer calls `fsync` after writing frames.
///
/// Choosing the right policy is a crash-safety vs. throughput tradeoff:
/// - `PerOp` fsyncs every frame *before* the write acks, so an acked write is
///   durable across an unclean `kill -9` (acked == durable). Highest per-write
///   latency: on macOS `sync_data` is a full `F_FULLFSYNC` device barrier.
/// - `Batched` (the default) acks after the frame is appended but fsyncs only on
///   a ~10ms group-commit timer or a 100-frame flush. Acked writes inside that
///   window are NOT durable under an unclean `kill -9`. This is the
///   throughput-favouring default for the single-node demo tier; set `PerOp`
///   when acked-implies-durable is required.
/// - `None` skips fsync entirely — useful for tests and throughput benchmarks
///   where crash-safety is not required.
///
/// The *behaviour* of the policy (actually calling fsync) is implemented in the
/// `WalWriter`. This enum is the configuration carrier only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WalFsyncPolicy {
    /// Call `fsync` after every appended frame, before the write acks. Acked ==
    /// durable under `kill -9`. Highest latency.
    PerOp,
    /// Default. Ack after append; fsync on a ~10ms group-commit timer / 100-frame
    /// flush. Highest throughput, but acked writes in the group-commit window are
    /// NOT durable under an unclean shutdown.
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
            "unknown WAL fsync policy {:?}; valid values are per_op (also perop, per-op), \
             batched, none (case-insensitive)",
            self.0
        )
    }
}

impl std::error::Error for ParseWalFsyncPolicyError {}

impl FromStr for WalFsyncPolicy {
    type Err = ParseWalFsyncPolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize so operator/harness/doc spellings all resolve: case-insensitive
        // and tolerant of the separator (`per_op`, `perop`, `per-op` are the same
        // intent). The soak harness and docs historically used the unseparated
        // `perop` spelling; rejecting it silently downgraded durability to the
        // Batched default, which is precisely the failure mode this normalization
        // prevents.
        let normalized = s.trim().to_ascii_lowercase().replace(['_', '-'], "");
        match normalized.as_str() {
            "perop" => Ok(Self::PerOp),
            "batched" => Ok(Self::Batched),
            "none" => Ok(Self::None),
            _ => Err(ParseWalFsyncPolicyError(s.trim().to_string())),
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
    /// Upsert: the full CRDT record value and its TTL expiration timestamp in
    /// milliseconds.
    Store {
        /// The CRDT value to persist. Carries the full [`RecordValue`] so OR-Map
        /// adds and tombstones survive `kill -9` recovery losslessly, while still
        /// decoding the bare-`Value` shape written by older servers.
        value: WalStorePayload,
        /// Wall-clock expiration time in milliseconds since epoch.
        /// Negative or zero means no expiration.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        expiration_time: Option<i64>,
    },
    /// Tombstone: remove the record from the store.
    Remove,
    /// Per-op OR-Map mutation. Recovery folds it onto the key's CURRENT durable
    /// value (never onto a WAL-window checkpoint) — see [`OrDeltaFold`].
    OrDelta {
        /// The single mutation this frame records.
        delta: crate::storage::wal::OrDelta,
    },
    /// A framing that carries only part of a key's state, so a successor CANNOT
    /// re-derive it.
    ///
    /// [`WalOp::OrDelta`] now answers the coalesce predicate's `false` branch on
    /// the production side, but this variant keeps two roles that no production
    /// variant can take over:
    ///
    /// - It is the only `WalOp` with no replay semantics — its
    ///   [`WalRecovery::prepare_entry`] arm returns an error rather than applying
    ///   anything — so it is what keeps recovery's behaviour on a frame kind it
    ///   cannot apply under test. `OrDelta` cannot fill that role: it folds
    ///   successfully.
    /// - It keeps the survivor-side mixed case (`snapshot_ops_subsume_on_coalesce`)
    ///   able to pair a subsuming op against a non-subsuming one WITHOUT binding
    ///   that proof to the OR variant's coalesce answer, so a later change to
    ///   that answer cannot silently hollow the mixed case out (`TG-OR-003`).
    ///
    /// Sited on this externally-tagged enum rather than on the `untagged`
    /// [`WalStorePayload`], where an extra arm would shift untagged decode
    /// ordering between test and production builds and corrupt the WAL
    /// round-trip proofs that run under `cfg(test)`.
    #[cfg(test)]
    TestNonSubsuming,
}

impl WalOp {
    /// Whether THIS op, as the SURVIVOR of a coalesce, makes an earlier frame for
    /// the same key redundant: the survivor carries the key's COMPLETE state, so
    /// dropping any predecessor loses nothing. Full snapshots and whole-key
    /// tombstones subsume; per-op deltas do not, because each delta carries
    /// independent information a successor cannot re-derive.
    ///
    /// The match is exhaustive with NO catch-all at any level, and that is the
    /// point: a future delta-framing variant cannot be added without deciding its
    /// answer here, so the coalesce branch cannot silently keep early-resolving
    /// frames that are no longer redundant.
    #[must_use]
    pub fn subsumes_on_coalesce(&self) -> bool {
        match self {
            // A whole-key tombstone IS complete state: it fully determines the key.
            Self::Remove => true,
            Self::Store { value, .. } => match value {
                // Full CRDT snapshot; a bare legacy value is also a complete state.
                WalStorePayload::Record(_) | WalStorePayload::Legacy(_) => true,
            },
            // Partial state: a per-op delta carries only its own mutation, so
            // dropping the predecessor would lose information this survivor
            // cannot re-derive. The retired sequences must ride along via the
            // carry-forward route instead of resolving early.
            Self::OrDelta { .. } => false,
            // Partial state: dropping the predecessor would lose information the
            // survivor does not carry, so the retired sequences must ride along
            // instead of resolving early.
            #[cfg(test)]
            Self::TestNonSubsuming => false,
        }
    }
}

/// Payload of a [`WalOp::Store`] frame.
///
/// Current servers write `Record(RecordValue)`, preserving the exact CRDT shape
/// (LWW, OR-Map, or legacy OR tombstones). Older servers wrote a bare
/// [`Value`] (and destroyed OR payloads as `Value::Null`); those frames still
/// decode through the `Legacy` arm and replay as LWW.
///
/// The enum is `untagged` so the same `value` field decodes both shapes. A
/// legacy frame's `value` holds an externally-tagged `Value` (e.g. `{"Int": …}`,
/// or the bare string `"Null"`), whose tags are disjoint from the
/// externally-tagged `RecordValue` tags (`lww`/`orMap`/`orTombstones`). The
/// `Record` arm therefore rejects legacy frames and the `Legacy` arm accepts
/// them, so the server never refuses to start on a pre-existing WAL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WalStorePayload {
    /// Full CRDT record value written by current servers.
    Record(RecordValue),
    /// Bare value written by older servers; replayed as an LWW record.
    Legacy(Value),
}

// ---------------------------------------------------------------------------
// OrDelta — per-op OR-Map mutation seam (recovery-read side)
// ---------------------------------------------------------------------------

/// The minimal per-op OR-Map mutation an OR write would append **instead of** a
/// full [`RecordValue::OrMap`] snapshot.
///
/// Today every `OR_ADD` / `OR_REMOVE` persists the *entire* per-key OR-Map slot
/// (`records: Vec<OrMapEntry>` + `tombstones: Vec<String>`) on every op, so the
/// Nth op on a key appends an O(N) frame and the retained WAL grows without
/// bound under sustained churn. Appending only the mutation makes the durable
/// append O(1) per op; recovery folds the deltas back onto the resident slot
/// (see [`OrDeltaFold`]).
///
/// The shape is an **enum discriminant** rather than a bag of `Option`s
/// (`{ added: Option<OrMapEntry>, removed_tag: Option<String>, pruned_tags:
/// Vec<String> }`): a single OR op is exactly one of add / remove, and an
/// epoch-GC sweep is a prune — an enum makes the illegal "both added and removed
/// set" state unrepresentable and satisfies the op-kind-discriminant
/// type-mapping rule (serde owns the tag; there is no
/// `r#type` field). The set-algebra a fold must apply matches the resident CRDT
/// exactly: add-wins (retain concurrent survivors, an already-tombstoned tag is
/// never resurrected), remove-wins (drop the matched tag from records, record its
/// tombstone), prune (drop the given tombstone tags once the low-water-mark has
/// passed their epoch).
///
/// ## Codec-safety envelope (design contract for the write-path wiring)
///
/// An `OrDelta` is carried inside a [`WalEntry`] and encoded through the SAME
/// [`format::encode`] path as every other frame, so it inherits the frame codec
/// invariants unchanged and MUST NOT weaken them:
///
/// - **CRC32C**: the frame stays length-prefixed and Castagnoli-checksummed; a
///   bit-flip in a delta frame is caught exactly as it is for a Store frame.
/// - **Length bound**: a delta payload is far smaller than a full snapshot, so it
///   is trivially within [`format::MAX_FRAME_PAYLOAD_LEN`]; the bound still
///   applies and rejects a corrupt/bloated declared length before allocating.
/// - **Exhaustive decode**: recovery MUST continue to pattern-match every
///   [`format::FrameDecodeResult`] variant and NEVER panic on malformed input —
///   folding a decoded delta happens only on the `Complete` / tolerated
///   `TruncatedTail` arms, exactly like the current Store replay.
/// - **Version discipline**: an `OrDelta`-carrying `WalOp` variant is an additive
///   MsgPack-named change inside the UNCHANGED frame envelope, so
///   [`format::FRAME_VERSION`] stays where it is — a bump is out of contract
///   until a dedicated migration owns it (TODO-617). A bump would not be a
///   stronger signal but an outage in both directions: `decode_all` treats ANY
///   version mismatch as fail-closed, so a bumped binary would refuse every
///   legacy WAL already on disk and an older binary would refuse every WAL
///   written after the bump.
///
///   What a LEGACY binary does with a delta frame it cannot deserialize has TWO
///   cases, decided purely by the frame's POSITION, and the unqualified
///   "always fails closed" reading is FALSE — writing it here would be a false
///   invariant in code:
///   - **mid-segment** (not the last frame in the byte stream) ⇒
///     [`format::FrameDecodeResult::Corruption`] ⇒ genuine refusal to start;
///   - **trailing frame on the ACTIVE segment** ⇒
///     [`format::FrameDecodeResult::TruncatedTail`], which recovery TOLERATES on
///     the last segment: it warns, replays the intact prefix, and `WalWriter`'s
///     open path then DURABLY TRUNCATES the segment to that prefix. The mutation
///     is silently dropped and physically gone, not merely skipped — and the
///     newest frame is the most likely position for the newest mutation.
///
///   That asymmetry is LIVE from this merge on: the write-behind store frames
///   per-op deltas, so delta-bearing WALs exist on disk and a node rolled back
///   across this commit can meet one. Rolling back may therefore silently drop
///   the newest OR mutation on each partition's active segment, and durably
///   truncate that segment to the intact prefix. See
///   [`format::FRAME_VERSION`] for the full record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrDelta {
    /// `OR_ADD`: a tagged entry was added. Fold: remove any existing entry with the
    /// same tag (idempotent re-add), then append this one — UNLESS the tag is
    /// already tombstoned, in which case the add is suppressed (remove-wins).
    Add {
        /// The added entry (value + unique tag + HLC timestamp).
        entry: OrMapEntry,
    },
    /// `OR_REMOVE`: a tag was observed-removed. Fold: drop the matched tag from
    /// `records` (preserving every concurrent survivor) and append it to
    /// `tombstones` if genuinely new.
    Remove {
        /// The observed-removed tag.
        tag: String,
    },
    /// Epoch-GC prune: the given tombstone tags are dropped once the fleet-wide
    /// low-water-mark has passed their epoch. Fold: remove these tags from
    /// `tombstones`.
    Prune {
        /// Tombstone tags being reclaimed. An empty `tags` is a representable
        /// no-op (a fold over it changes nothing); the write path MUST NOT emit
        /// one — an empty-prune delta only inflates the WAL without effect.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
    },
}

/// Recovery-fold interface: applies ONE [`OrDelta`] to an OR-Map slot,
/// reconstructing the value the full-snapshot path would have produced. The
/// returned value is a `RecordValue::OrMap`.
///
/// `base` is the key's CURRENT DURABLE STORE value — always, with no exception.
/// `None` means the store holds no record for the key at all, which is a
/// legitimate first write rather than an error; it never means "the replay
/// window opened on a checkpoint". Recovery replays the WHOLE un-applied window
/// (every entry above the partition's applied watermark) in strict sequence
/// order, writing each folded result back before the next frame for the key
/// reads it, so the DURABLE STORE is the accumulator.
///
/// **No frame is a skip hint.** A full-snapshot `WalOp::Store` OR frame inside
/// the window is applied in its own sequence position as an ABSOLUTE SET, like
/// every other frame — never consulted as a base the fold starts FROM. Reading
/// one as a base would discard its content whenever the coalesced store write
/// had not yet landed, silently losing every add it carried; and it could not
/// help in the other direction either, since nothing at or below the watermark
/// is ever a recovery input (`TG-OR-003`).
///
/// ## Recovery-equivalence invariant (SEMANTIC-SET, not byte-for-byte)
///
/// The reconstructed slot is required to be **semantic-set equivalent** to the
/// full-snapshot path's slot — the same live `(tag, value)` set, the same
/// tombstone set, and the same pruned-tag set — NOT byte-for-byte identical. The
/// resident `RecordValue::OrMap` stores `records`/`tombstones` in
/// **operation-insertion order** (the CRDT write path does `retain(tag != ...)`
/// then `push(...)`; it never canonically sorts), and cross-node OR-Map
/// convergence is set-based, so no canonical byte ordering exists to make
/// bit-equality a robust invariant. The differential recovery test asserts
/// this semantic-set equivalence, via the equivalence oracle defined alongside
/// the OR write path.
pub trait OrDeltaFold {
    /// Apply `delta` to `base`, returning the reconstructed
    /// `RecordValue::OrMap`.
    ///
    /// ONE frame per call, deliberately: a single-delta parameter is what stops
    /// a key's frames from being collected out of the replay stream and folded
    /// as a batch, which is the window-fold shape this trait no longer has. The
    /// add-wins / remove-wins / prune algebra is not re-stated by an
    /// implementor — it is the same live apply the resident CRDT slot uses.
    fn fold(base: Option<RecordValue>, delta: &OrDelta) -> RecordValue;
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
    /// HLC timestamp from the originating write. It is the idempotency key for
    /// `WalOp::Store` `Lww` frames ONLY: during recovery a replayed Store whose
    /// timestamp is older than the current durable value is a no-op (TG-WAL-006).
    /// `WalOp::Remove` frames carry `None` — a Remove replays UNCONDITIONALLY,
    /// exactly as the live remove path deletes unconditionally (no timestamp
    /// compare), so recovery never consults a Remove's timestamp (TG-WAL-009). A
    /// Remove gate would be unsound: live-remove has no such condition, and under
    /// prefix-complete in-order replay a stale Remove can never be juxtaposed over
    /// a strictly-newer re-creation (proof recorded in INVARIANTS.md TG-WAL-009).
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
/// A file-backed production implementation (`WalWriter`) and an in-memory
/// simulation double share this trait so callers can depend on `Arc<dyn Wal>`
/// without coupling to a specific implementation.
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

    /// Advances the partition's applied watermark to `sequence`, removing every
    /// entry at or below it from what `unapplied` returns.
    ///
    /// `sequence` MUST be **prefix-complete**: every frame at or below it has
    /// been durably applied to the inner store. It is not a high-water mark of
    /// whatever happened to succeed — a single un-applied frame holds the
    /// watermark below itself, because the watermark is what licenses the log to
    /// discard the frames it covers. Implementations clamp monotonically, so a
    /// caller that under-advances is safe; a caller that over-advances loses
    /// data.
    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()>;

    /// Returns all un-applied entries for a partition, in ascending sequence
    /// order.
    ///
    /// Startup replays these: they are the frames appended before the last crash
    /// that the watermark does not yet cover, so their effects may be missing
    /// from the inner store. Because the watermark is prefix-complete, this is a
    /// contiguous suffix of the log rather than an arbitrary subset. An empty vec
    /// means the partition is clean.
    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>>;

    /// Makes every frame appended so far durable, whatever the fsync policy
    /// has deferred.
    ///
    /// For the one caller that cannot leave durability to the policy: a store
    /// that is shutting down. Under a deferred policy the frames of its last
    /// moments are in the file but not yet fsynced, and the task that would
    /// fsync them ends with the process.
    ///
    /// The default does nothing, which is right for an implementation that
    /// defers no fsync or keeps no file.
    async fn sync(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// The durable applied watermark of `partition` as the store holds it, or
    /// `None` for a partition this WAL does not have.
    ///
    /// The watermark is kept by the store the WAL was opened over, committed
    /// together with the data: a caller commits each watermark row in the
    /// store batch that makes its frames durable, and
    /// [`mark_applied`](Wal::mark_applied) only records that the commit
    /// returned.
    fn durable_applied(&self, partition: u32) -> Option<u64>;

    /// Ends the WAL's background collection and, when `write_hints`, writes
    /// one rollback hint per partition: its durable applied watermark, in the
    /// sidecar format an older binary reads.
    ///
    /// Returns the number of hints written. The default writes none.
    async fn finish(&self, write_hints: bool) -> anyhow::Result<usize> {
        let _ = write_hints;
        Ok(0)
    }
}

// ===========================================================================
// WalWriter — production file-backed implementation
// ===========================================================================

use crate::storage::wal::segment::{Segment, SegmentCipher, SegmentHeader};

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
    /// Bytes appended to the active segment, so the collection tick can seal
    /// it by size. Changed only under the `active` lock.
    active_bytes: AtomicU64,
    /// When the first frame of the active segment was appended, so the
    /// collection tick can seal it by age. Changed only under the `active`
    /// lock.
    active_first_append: std::sync::Mutex<Option<std::time::Instant>>,
    /// The key of the sealed active segment, with the header it was derived
    /// from, so an append derives it once per segment. Used only under the
    /// `active` lock.
    active_key: std::sync::Mutex<Option<(SegmentHeader, crate::storage::sealing::SegmentKey)>>,
}

impl PartitionHandle {
    /// Counts a frame of `len` bytes appended to the active segment. Called
    /// under the `active` lock.
    fn note_append(&self, len: usize) {
        if self.active_bytes.fetch_add(len as u64, Ordering::Relaxed) == 0 {
            *self
                .active_first_append
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(std::time::Instant::now());
        }
    }

    /// Forgets the counts of a segment that was just sealed. Called under the
    /// `active` lock.
    fn note_rotation(&self) {
        self.active_bytes.store(0, Ordering::Relaxed);
        *self
            .active_first_append
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// The collection tick of a WAL: the only code that seals or unlinks a
/// segment.
struct Collection {
    /// The collected-up-to value of every partition, as `wal-collected`
    /// holds it. Its lock is the collection lock: a tick runs under it, so
    /// two ticks never interleave, and `finish` takes it to wait for a
    /// running tick to end.
    collected: AsyncMutex<Box<[u64]>>,
    /// Set by `finish`: no tick seals, writes or unlinks anything afterwards,
    /// and the task ends at its next wake.
    closed: AtomicBool,
    /// Wakes the tick task before its interval has passed.
    wake: Arc<tokio::sync::Notify>,
    /// The tick task, once spawned.
    task: std::sync::OnceLock<tokio::task::JoinHandle<()>>,
}

/// How often the collection tick runs.
const COLLECTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
/// Bytes appended to an active segment that make the tick seal it.
const SEAL_BYTES: u64 = 8 * 1024 * 1024;
/// Age of an active segment's first frame that makes the tick seal it, before
/// the partition's offset (see `seal_age`).
const SEAL_AGE: std::time::Duration = std::time::Duration::from_secs(30);

/// Age of `partition`'s first frame that makes the tick seal it: `SEAL_AGE`
/// plus an offset that spreads the partitions evenly over one more `SEAL_AGE`,
/// so every partition seals between 30 and 60 s. Partitions whose first frames
/// arrive together would otherwise all seal in one tick, and each seal fsyncs
/// under the partition's append lock: on a device whose fsync flushes its whole
/// cache, that is hundreds of flushes back to back while appends wait.
fn seal_age(partition: u32) -> std::time::Duration {
    SEAL_AGE + SEAL_AGE * partition / PARTITION_COUNT
}

/// File name of the collected-up-to record of a store-mode WAL directory.
pub(crate) const WAL_COLLECTED_FILE: &str = "wal-collected";
/// File name the collected-up-to record is written to before its rename.
pub(crate) const WAL_COLLECTED_TMP_FILE: &str = "wal-collected.tmp";
/// Magic of the collected-up-to record.
pub(crate) const WAL_COLLECTED_MAGIC: &[u8; 8] = b"TGWALC01";
/// Length of the collected-up-to record: magic, one u64 BE per partition,
/// crc32c u32 BE of the preceding bytes.
pub(crate) const WAL_COLLECTED_LEN: usize = 8 + 8 * PARTITION_COUNT as usize + 4;

/// Encodes the collected-up-to record: `TGWALC01 ‖ G(p) u64 BE for every
/// partition ‖ crc32c u32 BE`.
///
/// `collected` holds one value per partition (length `PARTITION_COUNT`); a
/// missing value is written as 0, so the record always has its fixed length.
pub(crate) fn encode_wal_collected(collected: &[u64]) -> Vec<u8> {
    debug_assert_eq!(collected.len(), PARTITION_COUNT as usize);
    let mut bytes = Vec::with_capacity(WAL_COLLECTED_LEN);
    bytes.extend_from_slice(WAL_COLLECTED_MAGIC);
    for partition in 0..PARTITION_COUNT as usize {
        bytes.extend_from_slice(&collected.get(partition).copied().unwrap_or(0).to_be_bytes());
    }
    let crc = crc32c::crc32c(&bytes);
    bytes.extend_from_slice(&crc.to_be_bytes());
    bytes
}

/// Decodes a collected-up-to record into one value per partition.
///
/// # Errors
///
/// A record of the wrong length, magic or checksum.
pub(crate) fn decode_wal_collected(bytes: &[u8]) -> anyhow::Result<Vec<u64>> {
    if bytes.len() != WAL_COLLECTED_LEN {
        anyhow::bail!(
            "it holds {} bytes, a collected-up-to record holds {WAL_COLLECTED_LEN}",
            bytes.len()
        );
    }
    let (body, crc) = bytes.split_at(WAL_COLLECTED_LEN - 4);
    if &body[..8] != WAL_COLLECTED_MAGIC {
        anyhow::bail!("it does not start with the magic of a collected-up-to record");
    }
    let stored = u32::from_be_bytes(crc.try_into()?);
    let computed = crc32c::crc32c(body);
    if stored != computed {
        anyhow::bail!("its checksum {stored:#010x} does not match its content ({computed:#010x})");
    }
    body[8..]
        .chunks_exact(8)
        .map(|chunk| Ok(u64::from_be_bytes(chunk.try_into()?)))
        .collect()
}

/// Writes the collected-up-to record of `wal_dir` crash-durably: a temporary
/// file, synced, renamed into place, then the directory synced.
///
/// The record licenses the absence of every frame it covers, so it must be
/// durable before any segment it covers is unlinked; a record that a crash
/// could tear or lose would let the next start take a data directory that
/// lacks those frames.
fn write_wal_collected(wal_dir: &Path, collected: &[u64]) -> anyhow::Result<()> {
    use std::io::Write as _;
    let tmp = wal_dir.join(WAL_COLLECTED_TMP_FILE);
    {
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| anyhow::anyhow!("Cannot create {}: {e}", tmp.display()))?;
        file.write_all(&encode_wal_collected(collected))
            .map_err(|e| anyhow::anyhow!("Cannot write {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| anyhow::anyhow!("Cannot fsync {}: {e}", tmp.display()))?;
    }
    let path = wal_dir.join(WAL_COLLECTED_FILE);
    std::fs::rename(&tmp, &path).map_err(|e| {
        anyhow::anyhow!("Cannot rename {} to {}: {e}", tmp.display(), path.display())
    })?;
    fsync_dir(wal_dir)
}

/// Production append-only WAL writer.
///
/// Each partition's log is split into a sequence of segment files named
/// `partition-{id:03}-{first_seq:020}.log` (see `segment`): appends always target
/// the single active segment. It is opened over a store
/// ([`WalWriter::open_over_store`]) that keeps the applied watermark: the
/// watermark of each partition is a row of that store, committed in the same
/// durable batch as the records it covers, so it can never be durable without
/// them. `mark_applied` only notes in memory that such a commit returned;
/// sealing and collection belong to the collection tick. The WAL directory
/// holds no watermark of its own, only `wal-collected`, the highest sequence of
/// each partition whose segments were unlinked — the check at open that the
/// data directory is not older than the WAL directory. The `.applied` files are
/// rollback hints for older binaries, written at a stop that exits 0; this
/// binary reads one only when the open imports the sidecars of an older binary,
/// and deletes them at every open.
///
/// Thread safety: each partition is guarded by its own `PartitionHandle` (split
/// active/sealed locks) so concurrent appends to different partitions do not block
/// each other and GC never serializes against appends.
pub struct WalWriter {
    wal_dir: PathBuf,
    policy: WalFsyncPolicy,
    /// Per-partition segment handles, lazily opened on first append/discovery.
    partitions: AsyncMutex<HashMap<u32, Arc<PartitionHandle>>>,
    /// Serializes the slow path of `handle`: one open per partition creates
    /// files, every later one finds its handle installed (`TG-ENC-005`). Lock
    /// order: the collection lock, then this, then a partition's active lock.
    open: AsyncMutex<()>,
    /// Set by a `finish` that writes rollback hints, under the open mutex and
    /// every installed active lock: from then on an append is refused before
    /// it frames anything, and no partition is opened. Separate from
    /// `collection.closed`, which every `finish` sets.
    appends_closed: AtomicBool,
    /// Batched-commit group-commit timer: shared notifier woken every ~10 ms.
    /// When `policy == Batched`, the writer also fsyncs after every 100 ops.
    batch_flush_tx: tokio::sync::watch::Sender<()>,
    /// The store that holds the watermark. Weak, so the WAL never keeps the
    /// store alive.
    store: std::sync::Weak<dyn MapDataStore>,
    /// The store's keyring, fixed for the life of the process: new segments
    /// are sealed under its active key, sealed segments are read with it
    /// (`TG-ENC-005`).
    keyring: Option<Arc<Keyring>>,
    /// The durable watermark of every WAL partition, indexed by partition:
    /// the value of its row once a commit carrying it returned. In memory
    /// only — the store's row is the durable copy.
    durable: Box<[AtomicU64]>,
    /// The collection tick's state.
    collection: Collection,
    /// Added to the clock the collection tick reads a segment's age by, so a
    /// test can age a segment without waiting.
    #[cfg(test)]
    test_clock_offset: std::sync::Mutex<std::time::Duration>,
    /// Per-partition value the collection tick uses instead of the store's
    /// watermark when it is lower, so a test can force a retention.
    #[cfg(test)]
    test_tick_watermark_override: std::sync::Mutex<HashMap<u32, u64>>,
    /// Per-partition floor `unapplied` filters by instead of the durable
    /// watermark, so a test can hand recovery a frame at or below it.
    #[cfg(test)]
    test_unapplied_floor_override: std::sync::Mutex<HashMap<u32, u64>>,
    /// Called with `(path, length)` after every successful fsync of an active
    /// segment, still under that segment's lock, so a crash test learns the
    /// last length of each file that a power loss would keep.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    test_fsync_observer: std::sync::Mutex<Option<Arc<dyn Fn(&Path, u64) + Send + Sync>>>,
    /// While set, the batched policy fsyncs no frame: the group-commit timer
    /// skips its round and the 100-frame fsync does not fire.
    #[cfg(test)]
    test_hold_batched_fsync: AtomicBool,
    /// Sealed segments the collection tick fails to unlink.
    #[cfg(test)]
    test_refused_unlinks: std::sync::Mutex<HashSet<PathBuf>>,
    /// Told of every segment created and every frame framed.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    test_observer: std::sync::Mutex<Option<Arc<dyn Fn(WalTestEvent) + Send + Sync>>>,
    /// Waited on by every open that missed the installed map, before it takes
    /// the open mutex.
    #[cfg(test)]
    test_open_barrier: std::sync::Mutex<Option<Arc<tokio::sync::Barrier>>>,
    /// Makes the next rotation fail as if the directory fsync after the new
    /// segment's rename had failed.
    #[cfg(test)]
    test_fail_rotation_dir_fsync: AtomicBool,
    /// Parks the next append to a partition at the entry of `append`.
    #[cfg(test)]
    test_append_park: std::sync::Mutex<Option<TestAppendPark>>,
}

/// A sealed segment was retained instead of reclaimed.
///
/// Typed rather than an `assert!` because a failed re-validation must surface as
/// a value the caller can count and log: aborting the process would turn a
/// conservative retention — which loses nothing — into an outage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WalGcError {
    /// The watermark re-read immediately before the unlink does not cover the
    /// segment that the locally computed watermark had licensed for reclamation.
    WatermarkBelowSegment {
        partition: u32,
        segment_max_seq: u64,
        revalidated: u64,
    },
}

impl std::fmt::Display for WalGcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WatermarkBelowSegment {
                partition,
                segment_max_seq,
                revalidated,
            } => write!(
                f,
                "refusing to unlink sealed WAL segment of partition {partition}: its highest \
                 sequence {segment_max_seq} is above the re-read applied watermark {revalidated}"
            ),
        }
    }
}

impl std::error::Error for WalGcError {}

/// Why a sealed segment was kept, as the `reason` label on the skipped-GC counter.
const GC_SKIP_REASON_WATERMARK: &str = "watermark_below_segment";

/// Per-partition lag between the highest assigned WAL sequence and the applied
/// watermark — the continuous early warning under the fatal alarm.
///
/// Sited here rather than beside its only producer because the skipped-GC
/// counter below MUST be shared by two producers in different modules, and
/// splitting the pair would leave one of them describing a metric it does not
/// emit.
pub(crate) const WAL_WATERMARK_LAG_GAUGE: &str = "topgun_wal_applied_watermark_lag";

/// Sealed WAL segments retained rather than reclaimed, by reason.
pub(crate) const WAL_GC_SKIPPED_COUNTER: &str = "topgun_wal_gc_skipped_total";

/// Bytes of WAL segments a partition retains, by `partition`.
pub(crate) const WAL_RETAINED_BYTES_GAUGE: &str = "topgun_wal_retained_bytes";

/// Bytes of WAL segments all partitions retain.
pub(crate) const WAL_RETAINED_BYTES_TOTAL_GAUGE: &str = "topgun_wal_retained_bytes_total";

/// Why a collection tick collected nothing, as the `reason` label on the
/// skipped-GC counter: the store's watermarks could not be read.
const GC_SKIP_REASON_READ_FAILED: &str = "watermark_read_failed";

/// Sets the retained-bytes gauge of every partition, 0 where `by_partition`
/// has none, and their total.
///
/// Every partition is set, so each series renders from the first scrape on
/// rather than appearing only once its partition holds a segment.
fn set_retained_gauges(by_partition: &BTreeMap<u32, u64>) {
    metrics::describe_gauge!(
        WAL_RETAINED_BYTES_GAUGE,
        "Bytes of WAL segments a partition retains: its sealed segments and its active one"
    );
    metrics::describe_gauge!(
        WAL_RETAINED_BYTES_TOTAL_GAUGE,
        "Bytes of WAL segments all partitions retain"
    );
    let mut total = 0u64;
    for partition in 0..PARTITION_COUNT {
        let bytes = by_partition.get(&partition).copied().unwrap_or(0);
        total = total.saturating_add(bytes);
        // Precision loss only past 2^53 bytes, far beyond any disk.
        #[allow(clippy::cast_precision_loss)]
        metrics::gauge!(WAL_RETAINED_BYTES_GAUGE, "partition" => partition.to_string())
            .set(bytes as f64);
    }
    #[allow(clippy::cast_precision_loss)]
    metrics::gauge!(WAL_RETAINED_BYTES_TOTAL_GAUGE).set(total as f64);
}

/// Registers HELP/TYPE metadata for the two WAL-watermark metrics.
///
/// Idempotent, and called from every emission path rather than from one
/// initialisation point: the recorder is process-global and shared by every
/// producer, so a describe tied to a single construction site would leave the
/// metrics undescribed for whichever producer ran first.
///
/// Deliberately NOT `Once`-guarded. A one-shot describe is only registered
/// against whichever recorder happens to be installed at the first emission —
/// an emission preceding `init_observability` would burn the guard on the no-op
/// recorder and leave the metrics permanently HELP-less on `/metrics`. The
/// `describe_*` macros are idempotent metadata registrations and the emission
/// paths here are a per-tick watchdog sample and a rare retained-segment GC
/// skip, never a per-write hot path.
pub(crate) fn describe_wal_watermark_metrics() {
    metrics::describe_gauge!(
        WAL_WATERMARK_LAG_GAUGE,
        "WAL sequences assigned for a partition but not yet covered by its applied watermark"
    );
    metrics::describe_counter!(
        WAL_GC_SKIPPED_COUNTER,
        "Sealed WAL segments retained instead of reclaimed, labeled by the reason they were kept"
    );
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
    /// Opens a WAL in `wal_dir` whose applied watermark is kept by `store`,
    /// committed together with the data.
    ///
    /// The open checks that the data directory and the WAL directory belong
    /// together, imports the `.applied` sidecars of an older binary once, and
    /// starts the collection tick. Its steps, in order:
    ///
    /// 1. create the WAL directory;
    /// 2. read the store's watermarks, import marker and import-pending mark;
    /// 3. take the store's watermarks once imported; else read the sidecars —
    ///    only when the directory has never served a store-mode WAL (no
    ///    `wal-collected`) or the store was just migrated from the file of an
    ///    older binary — and refuse a data directory that holds no watermarks
    ///    beside a directory that has served a store keeping them;
    /// 4. refuse when any watermark is below what the WAL already collected;
    /// 5. import the sidecar values, with the marker, in one durable batch;
    /// 6. create `wal-collected`, all zero, when absent;
    /// 7. delete every sidecar, sidecar temporary file and segment temporary
    ///    file;
    /// 8. hold the watermarks in memory, set the retained-bytes gauges and
    ///    start the collection tick.
    ///
    /// Every refusal returns before anything is written to the store or to
    /// the WAL directory.
    ///
    /// # Errors
    ///
    /// A store that keeps no applied watermark; a data directory older than
    /// the WAL directory; a malformed sidecar or collected-up-to record; an
    /// I/O or store error.
    // Owned, as the binary and every caller hold the store they open over;
    // the WAL keeps only a weak reference to it.
    #[allow(clippy::needless_pass_by_value)]
    pub fn open_over_store(
        wal_dir: PathBuf,
        policy: WalFsyncPolicy,
        store: Arc<dyn MapDataStore>,
    ) -> anyhow::Result<Arc<Self>> {
        if !store.keeps_applied_watermarks() {
            anyhow::bail!(
                "WAL directory {}: the store it is opened over keeps no applied watermark",
                wal_dir.display()
            );
        }
        Self::create_dir(&wal_dir)?;

        let watermarks = store
            .applied_watermarks()
            .map_err(|e| anyhow::anyhow!("Cannot read the applied watermarks of the store: {e:#}"))?
            .ok_or_else(|| anyhow::anyhow!("the store keeps no applied watermark"))?;
        if watermarks.by_partition.len() != PARTITION_COUNT as usize {
            anyhow::bail!(
                "the store returned {} applied watermarks, one per WAL partition is {PARTITION_COUNT}",
                watermarks.by_partition.len()
            );
        }

        let collected_present = Self::wal_collected_exists(&wal_dir)?;
        let effective = if watermarks.imported {
            watermarks.by_partition
        } else if !collected_present || watermarks.import_pending {
            Self::read_sidecars_for_import(&wal_dir)?
        } else {
            let message = format!(
                "WAL directory check: refusing to start: the data directory is older than the WAL \
                 directory {}: the data directory holds no applied watermarks, but the WAL \
                 directory has served a store that keeps them. Restore the data directory and the \
                 WAL directory from the same backup, taken with the server stopped",
                wal_dir.display()
            );
            tracing::error!("{message}");
            anyhow::bail!(message);
        };

        let collected = if collected_present {
            let path = wal_dir.join(WAL_COLLECTED_FILE);
            let bytes = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("Cannot read {}: {e}", path.display()))?;
            decode_wal_collected(&bytes).map_err(|e| {
                anyhow::anyhow!(
                    "WAL directory check: refusing to start: {} is malformed: {e}",
                    path.display()
                )
            })?
        } else {
            vec![0; PARTITION_COUNT as usize]
        };
        Self::refuse_collected_above_watermark(&wal_dir, &effective, &collected)?;

        if !watermarks.imported {
            let rows: Vec<(u32, u64)> = (0..PARTITION_COUNT)
                .zip(effective.iter().copied())
                .filter(|(_, sequence)| *sequence > 0)
                .collect();
            store.import_applied_watermarks(&rows).map_err(|e| {
                anyhow::anyhow!("Cannot import the applied watermarks into the store: {e:#}")
            })?;
        }

        if !collected_present {
            write_wal_collected(&wal_dir, &vec![0; PARTITION_COUNT as usize])?;
        }

        Self::delete_sidecars(&wal_dir)?;

        let durable: Box<[AtomicU64]> = effective.into_iter().map(AtomicU64::new).collect();
        let writer = Self::build(
            wal_dir,
            policy,
            Arc::downgrade(&store),
            store.wal_keyring(),
            durable,
            collected.into_boxed_slice(),
        );
        writer.start_collection()?;
        Ok(writer)
    }

    /// Sets the retained-bytes gauges of every partition and starts the
    /// collection tick.
    ///
    /// The task holds only weak references, as the batched timer does: the
    /// WAL and the store it reaches through the WAL stay droppable, and the
    /// task ends at its first wake after either is gone. Its first tick runs
    /// one interval after the open, not at once.
    fn start_collection(self: &Arc<Self>) -> anyhow::Result<()> {
        set_retained_gauges(&Self::segment_bytes_by_partition(&self.wal_dir)?);
        let writer = Arc::downgrade(self);
        let wake = Arc::clone(&self.collection.wake);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = tokio::time::sleep(COLLECTION_INTERVAL) => {}
                    () = wake.notified() => {}
                }
                let Some(writer) = writer.upgrade() else {
                    return;
                };
                if !writer.collect().await {
                    return;
                }
            }
        });
        // Set once: this is the only place that spawns the task, and it runs
        // once per writer.
        let _ = self.collection.task.set(task);
        Ok(())
    }

    /// The summed size of every segment file of each partition that has one.
    ///
    /// Read from the directory, so a partition whose segments this process
    /// has not opened yet — after a restart — is counted and collected too.
    fn segment_bytes_by_partition(wal_dir: &Path) -> anyhow::Result<BTreeMap<u32, u64>> {
        let mut bytes: BTreeMap<u32, u64> = BTreeMap::new();
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir {
            let entry = entry
                .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
            let Some((partition, _)) =
                segment::parse_segment_filename(&entry.file_name().to_string_lossy())
            else {
                continue;
            };
            // A segment unlinked between the listing and this read retains
            // nothing.
            let len = entry.metadata().map_or(0, |m| m.len());
            *bytes.entry(partition).or_default() += len;
        }
        Ok(bytes)
    }

    /// Creates `wal_dir` if it does not exist.
    fn create_dir(wal_dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(wal_dir).map_err(|e| {
            anyhow::anyhow!(
                "Cannot create WAL directory {}: {e}. \
                 Ensure the WAL dir is on the same volume as the durable store.",
                wal_dir.display()
            )
        })
    }

    /// Whether `wal_dir` holds a collected-up-to record, i.e. has served a
    /// store-mode WAL.
    fn wal_collected_exists(wal_dir: &Path) -> anyhow::Result<bool> {
        let path = wal_dir.join(WAL_COLLECTED_FILE);
        path.try_exists()
            .map_err(|e| anyhow::anyhow!("Cannot check {}: {e}", path.display()))
    }

    /// The value of every `partition-{p:03}.applied` sidecar of `wal_dir`, one
    /// per partition, 0 where there is none.
    ///
    /// A sidecar that is not exactly 8 bytes, or that names no WAL partition,
    /// refuses the start rather than reading as 0: imported as 0, its
    /// partition would replay frames the store already holds, from below the
    /// watermark the older binary had reached. Temporary files are ignored —
    /// an older binary renamed none of them into place.
    fn read_sidecars_for_import(wal_dir: &Path) -> anyhow::Result<Vec<u64>> {
        let mut values = vec![0u64; PARTITION_COUNT as usize];
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        for entry in read_dir {
            let entry = entry
                .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name
                .strip_prefix("partition-")
                .and_then(|rest| rest.strip_suffix(".applied"))
            else {
                continue;
            };
            let path = entry.path();
            let partition = id
                .parse::<u32>()
                .ok()
                .filter(|p| *p < PARTITION_COUNT)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "WAL directory check: refusing to start: {} names no WAL partition \
                         0..{PARTITION_COUNT}; its applied watermark cannot be imported",
                        path.display()
                    )
                })?;
            let bytes = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("Cannot read {}: {e}", path.display()))?;
            let bytes: [u8; 8] = bytes.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!(
                    "WAL directory check: refusing to start: {} holds {} bytes, an applied \
                     watermark is 8; it cannot be imported",
                    path.display(),
                    bytes.len()
                )
            })?;
            values[partition as usize] = u64::from_be_bytes(bytes);
        }
        Ok(values)
    }

    /// Refuses the start when the WAL has collected, in some partition, above
    /// the watermark the data directory holds: the frames in between are gone
    /// from the WAL and may be missing from the data directory.
    fn refuse_collected_above_watermark(
        wal_dir: &Path,
        watermarks: &[u64],
        collected: &[u64],
    ) -> anyhow::Result<()> {
        let mut behind = (0..PARTITION_COUNT)
            .zip(watermarks.iter().zip(collected))
            .filter(|(_, (v, g))| v < g);
        let Some((partition, (v, g))) = behind.next() else {
            return Ok(());
        };
        let more = behind.count();
        let more = if more == 0 {
            String::new()
        } else {
            format!(" (and {more} more partitions)")
        };
        let message = format!(
            "WAL directory check: refusing to start: the data directory is older than the WAL \
             directory {}: partition {partition} has applied watermark {v} there, but the WAL was \
             collected up to {g}{more}; frames {}..={g} are no longer in the WAL and may be \
             missing from the data directory. Restore the data directory and the WAL directory \
             from the same backup, taken with the server stopped",
            wal_dir.display(),
            v + 1
        );
        tracing::error!("{message}");
        anyhow::bail!(message)
    }

    /// Deletes every `.applied` sidecar, sidecar temporary file and segment
    /// temporary file of `wal_dir`, then makes the deletions durable.
    ///
    /// Run at every store-mode open, also once the import marker is present:
    /// a crash between the import and this step leaves them behind, and a
    /// sidecar left in place would be read by an older binary after a
    /// rollback as a watermark this binary no longer keeps.
    fn delete_sidecars(wal_dir: &Path) -> anyhow::Result<()> {
        let read_dir = std::fs::read_dir(wal_dir)
            .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
        let mut deleted = false;
        for entry in read_dir {
            let entry = entry
                .map_err(|e| anyhow::anyhow!("Cannot read WAL dir {}: {e}", wal_dir.display()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // A `.log.tmp` is a segment whose creation a crash cut before its
            // rename: it never held a frame, and deleting it before any
            // partition is opened keeps it from being taken for a segment.
            if !(name.starts_with("partition-")
                && (name.ends_with(".applied")
                    || name.ends_with(".applied.tmp")
                    || name.ends_with(".log.tmp")))
            {
                continue;
            }
            let path = entry.path();
            std::fs::remove_file(&path)
                .map_err(|e| anyhow::anyhow!("Cannot delete {}: {e}", path.display()))?;
            if !deleted {
                #[cfg(test)]
                test_abort_if_armed(TestAbortPoint::OpenInsideSidecarDelete);
                deleted = true;
            }
        }
        if deleted {
            fsync_dir(wal_dir)?;
        }
        Ok(())
    }

    /// Constructs the writer and, under the batched policy, its group-commit
    /// timer.
    fn build(
        wal_dir: PathBuf,
        policy: WalFsyncPolicy,
        store: std::sync::Weak<dyn MapDataStore>,
        keyring: Option<Arc<Keyring>>,
        durable: Box<[AtomicU64]>,
        collected: Box<[u64]>,
    ) -> Arc<Self> {
        let (batch_flush_tx, batch_flush_rx) = tokio::sync::watch::channel(());

        let writer = Arc::new(Self {
            wal_dir,
            policy,
            partitions: AsyncMutex::new(HashMap::new()),
            open: AsyncMutex::new(()),
            appends_closed: AtomicBool::new(false),
            batch_flush_tx,
            store,
            keyring,
            durable,
            collection: Collection {
                collected: AsyncMutex::new(collected),
                closed: AtomicBool::new(false),
                wake: Arc::new(tokio::sync::Notify::new()),
                task: std::sync::OnceLock::new(),
            },
            #[cfg(test)]
            test_clock_offset: std::sync::Mutex::new(std::time::Duration::ZERO),
            #[cfg(test)]
            test_tick_watermark_override: std::sync::Mutex::new(HashMap::new()),
            #[cfg(test)]
            test_unapplied_floor_override: std::sync::Mutex::new(HashMap::new()),
            #[cfg(test)]
            test_fsync_observer: std::sync::Mutex::new(None),
            #[cfg(test)]
            test_hold_batched_fsync: AtomicBool::new(false),
            #[cfg(test)]
            test_refused_unlinks: std::sync::Mutex::new(HashSet::new()),
            #[cfg(test)]
            test_observer: std::sync::Mutex::new(None),
            #[cfg(test)]
            test_open_barrier: std::sync::Mutex::new(None),
            #[cfg(test)]
            test_fail_rotation_dir_fsync: AtomicBool::new(false),
            #[cfg(test)]
            test_append_park: std::sync::Mutex::new(None),
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
                    #[cfg(test)]
                    if w.test_hold_batched_fsync.load(Ordering::Acquire) {
                        continue;
                    }
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
                            match active.sync_data().await {
                                Ok(()) => {
                                    #[cfg(test)]
                                    w.test_observe_fsync(&active);
                                }
                                Err(e) => tracing::error!(
                                    segment = ?active.path(),
                                    error = %e,
                                    "WAL batch fsync failed; acked frames may not be durable"
                                ),
                            }
                        }
                    }
                }
            });
        }

        writer
    }

    /// The durable watermark slot of `partition`.
    fn durable_slot(durable: &[AtomicU64], partition: u32) -> anyhow::Result<&AtomicU64> {
        durable.get(partition as usize).ok_or_else(|| {
            anyhow::anyhow!("WAL partition {partition} is outside 0..{PARTITION_COUNT}")
        })
    }

    /// Path of the active segment for a fresh partition (seeded at `first_seq`).
    fn first_segment_path(&self, partition: u32) -> PathBuf {
        self.wal_dir
            .join(segment::format_segment_filename(partition, 0))
    }

    /// Path of the rollback hint that records a partition's applied watermark
    /// in the format an older binary reads.
    fn applied_path(&self, partition: u32) -> PathBuf {
        self.wal_dir
            .join(format!("partition-{partition:03}.applied"))
    }

    /// Confirms a re-read watermark still covers a segment about to be unlinked.
    ///
    /// A typed `Err` rather than an `assert!`: retaining a segment costs disk and
    /// nothing else, so the conservative outcome must stay reachable instead of
    /// taking the process down.
    fn revalidate_before_unlink(
        partition: u32,
        segment_max_seq: u64,
        revalidated: u64,
    ) -> Result<(), WalGcError> {
        if segment_max_seq > revalidated {
            return Err(WalGcError::WatermarkBelowSegment {
                partition,
                segment_max_seq,
                revalidated,
            });
        }
        Ok(())
    }

    /// Writes `seq` to the `.applied` file at `path` as a big-endian u64: a
    /// temporary file, synced, renamed into place. The directory is not
    /// synced, so a caller writing several files syncs it once.
    fn write_applied_file(path: &Path, seq: u64) -> anyhow::Result<()> {
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

    /// Creates the active segment of `partition` at `path`, starting at
    /// `first_seq`, with a fresh header (`TG-ENC-003`): sealed under the
    /// keyring's active key when it has one, else unsealed.
    fn create_active(
        &self,
        path: PathBuf,
        partition: u32,
        first_seq: u64,
    ) -> anyhow::Result<Segment> {
        let mut header = SegmentHeader::new_unsealed(partition, first_seq);
        if let Some(key_id) = self.keyring.as_deref().and_then(Keyring::active_id) {
            header.cipher = SegmentCipher::XChaCha20Poly1305;
            header.key_id = key_id;
        }
        let file = segment::create_segment_file(&path, &header)?;
        #[cfg(test)]
        self.test_observe(WalTestEvent::Created {
            partition,
            path: path.clone(),
            salt: header.salt,
        });
        Ok(Segment::new_active(
            path,
            header,
            tokio::fs::File::from_std(file),
        ))
    }

    /// Returns the live `PartitionHandle`, opening it from disk on first use.
    ///
    /// The open never appends to a file it did not create (`TG-ENC-005`): a
    /// frame written by an earlier process may sit past the persisted end of
    /// its file, and a reissued sequence may follow it, so a position of an
    /// earlier file is never written again. The partition's segment files are
    /// discovered by filename (ascending `first_seq`) and all kept as sealed,
    /// except a last file that holds no intact frame, which is replaced by a
    /// new file under its name. A last file with frames is first cut to its
    /// intact prefix, since it becomes non-last; a new active segment then
    /// starts at its highest sequence plus one. A partition with no segment
    /// starts at `first_seq = 0`.
    ///
    /// Opens are serialized by the open mutex and re-check the installed map
    /// under it, so exactly one handle is installed per partition and no file
    /// of a partition with an installed handle is created, cut or replaced.
    async fn handle(&self, partition: u32) -> anyhow::Result<Arc<PartitionHandle>> {
        if let Some(h) = self.partitions.lock().await.get(&partition) {
            return Ok(Arc::clone(h));
        }
        #[cfg(test)]
        self.test_wait_open_barrier().await;
        let _open = self.open.lock().await;
        if self.appends_closed.load(Ordering::Acquire) {
            anyhow::bail!("WAL partition {partition} not opened: the WAL is closed");
        }
        if let Some(h) = self.partitions.lock().await.get(&partition) {
            return Ok(Arc::clone(h));
        }

        let mut segments = Self::discover_partition_segments(&self.wal_dir, partition)?;
        let mut sealed: Vec<Segment> = Vec::new();
        let active = if let Some((last_first_seq, last_path)) = segments.pop() {
            for (first_seq, path) in segments {
                let max_seq = self.decode_segment_max_seq(&path).await?;
                sealed.push(Segment::sealed_existing(path, first_seq, max_seq));
            }
            let data = tokio::fs::read(&last_path).await.map_err(|e| {
                anyhow::anyhow!("Cannot read WAL segment {}: {e}", last_path.display())
            })?;
            let (entries, intact_len) = self.decode_segment_keyed(&last_path, &data, true)?;
            if let Some(max_seq) = entries.iter().map(|e| e.sequence).max() {
                // Cut a torn tail durably before the file becomes non-last:
                // left in place, the torn bytes would be mid-partition
                // corruption that refuses the next recovery.
                if intact_len < data.len() {
                    tracing::warn!(
                        path = ?last_path,
                        file_len = data.len(),
                        intact_len,
                        "WAL last segment has a torn tail; truncating to intact prefix"
                    );
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&last_path)
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Cannot open WAL segment {} for truncation: {e}",
                                last_path.display()
                            )
                        })?;
                    file.set_len(intact_len as u64).map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot truncate torn WAL segment {}: {e}",
                            last_path.display()
                        )
                    })?;
                    file.sync_all().map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot fsync truncated WAL segment {}: {e}",
                            last_path.display()
                        )
                    })?;
                }
                sealed.push(Segment::sealed_existing(last_path, last_first_seq, max_seq));
                let first_seq = max_seq.saturating_add(1);
                let path = self
                    .wal_dir
                    .join(segment::format_segment_filename(partition, first_seq));
                self.create_active(path, partition, first_seq)?
            } else {
                self.create_active(last_path, partition, last_first_seq)?
            }
        } else {
            self.create_active(self.first_segment_path(partition), partition, 0)?
        };

        let handle = Arc::new(PartitionHandle {
            active: AsyncMutex::new(active),
            sealed: AsyncMutex::new(sealed),
            active_bytes: AtomicU64::new(0),
            active_first_append: std::sync::Mutex::new(None),
            active_key: std::sync::Mutex::new(None),
        });
        self.partitions
            .lock()
            .await
            .insert(partition, Arc::clone(&handle));
        Ok(handle)
    }

    /// Decodes the highest sequence number present in a segment file, tolerating a
    /// truncated tail on the file (returns the intact prefix's max) and refusing on
    /// mid-file corruption. Returns the file's `first_seq` floor when it is empty —
    /// the caller distinguishes empty from a genuine single-frame segment via the
    /// frame count, not this value.
    async fn decode_segment_max_seq(&self, path: &Path) -> anyhow::Result<u64> {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| anyhow::anyhow!("Cannot read WAL segment {}: {e}", path.display()))?;
        let (entries, _) = self.decode_segment_keyed(path, &data, true)?;
        Ok(entries.iter().map(|e| e.sequence).max().unwrap_or(0))
    }

    /// Decodes every frame in one segment's bytes, applying the shared corruption
    /// taxonomy, and returns them with the length of the intact prefix.
    /// `tolerate_truncated_tail` controls whether a truncated tail is a
    /// recoverable WARN (active / last segment) or a fatal refusal (a sealed,
    /// non-last segment must never be torn). A header that names another
    /// partition or first sequence than the file name, or a refused header, is
    /// a refusal. A sealed segment is opened with the WAL's keyring; one whose
    /// key id the keyring lacks, or a frame that fails authentication, is a
    /// refusal naming the segment (`TG-ENC-003`). Every production reader of
    /// segment frames reads through this.
    fn decode_segment_keyed(
        &self,
        path: &Path,
        data: &[u8],
        tolerate_truncated_tail: bool,
    ) -> anyhow::Result<(Vec<WalEntry>, usize)> {
        Self::entries_or_refuse(
            path,
            format::decode_segment_with(data, self.keyring.as_deref()),
            tolerate_truncated_tail,
        )
    }

    /// As [`Self::decode_segment_keyed`] with no keyring: a sealed segment is
    /// refused.
    #[cfg(test)]
    fn decode_segment_or_refuse(
        path: &Path,
        data: &[u8],
        tolerate_truncated_tail: bool,
    ) -> anyhow::Result<(Vec<WalEntry>, usize)> {
        Self::entries_or_refuse(path, format::decode_segment(data), tolerate_truncated_tail)
    }

    fn entries_or_refuse(
        path: &Path,
        decoded: format::SegmentDecode,
        tolerate_truncated_tail: bool,
    ) -> anyhow::Result<(Vec<WalEntry>, usize)> {
        if let Some(header) = &decoded.header {
            let named = path
                .file_name()
                .and_then(|name| segment::parse_segment_filename(&name.to_string_lossy()));
            if named != Some((header.partition, header.first_seq)) {
                anyhow::bail!(
                    "WAL segment {} has a header for partition {} from sequence {}, which its \
                     name does not carry; refusing to start",
                    path.display(),
                    header.partition,
                    header.first_seq
                );
            }
        }
        let entries = match decoded.result {
            format::FrameDecodeResult::Complete(entries) => entries,
            format::FrameDecodeResult::CleanEof => Vec::new(),
            format::FrameDecodeResult::TruncatedTail { complete } => {
                if tolerate_truncated_tail {
                    tracing::warn!(
                        path = ?path,
                        "WAL tail truncated (crash mid-write); replaying intact prefix"
                    );
                    complete
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
            format::FrameDecodeResult::BadHeader(error) => {
                anyhow::bail!(
                    "WAL segment {} has a header that is refused: {error}; refusing to start",
                    path.display()
                );
            }
            format::FrameDecodeResult::Sealed { key_id } => {
                anyhow::bail!(
                    "WAL segment {} is sealed with data key {key_id}, which is not in the \
                     keyring; refusing to start",
                    path.display()
                );
            }
            format::FrameDecodeResult::AuthenticationFailed { offset } => {
                anyhow::bail!(
                    "WAL segment {} has a frame at offset {offset} that fails authentication: \
                     its bytes were changed; refusing to start",
                    path.display()
                );
            }
        };
        Ok((entries, decoded.intact_len))
    }
}

impl WalWriter {
    /// The frame of `entry` for `partition`'s active segment `active`, and
    /// its index. Called under the active-append lock: the frame is encoded
    /// and sealed there, and its index taken only once the payload has
    /// encoded within the frame limit, so the index is always the frame's
    /// position in the segment, is the nonce of its seal and is never handed
    /// out twice (`TG-ENC-005`).
    ///
    /// # Errors
    ///
    /// An entry that does not encode within the frame limit; no index is
    /// taken then. Every later failure stops the process (`TG-WAL-007`).
    fn frame_under_lock(
        &self,
        partition: u32,
        handle: &PartitionHandle,
        active: &mut Segment,
        entry: &WalEntry,
    ) -> anyhow::Result<(Vec<u8>, u64)> {
        let sealed_header = active
            .header()
            .filter(|header| header.cipher != SegmentCipher::None)
            .copied();
        let mut active_key = handle
            .active_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = sealed_header.map(|header| {
            if active_key.as_ref().map(|(of, _)| of) != Some(&header) {
                let key = self
                    .keyring
                    .as_deref()
                    .ok_or(crate::storage::sealing::UnknownSegmentKey(header.key_id))
                    .and_then(|keyring| keyring.segment_key(&header))
                    .unwrap_or_else(|e| {
                        wal_fail_stop(
                            WalFailStopTier::P,
                            &format!(
                                "the active segment of partition={partition} is sealed with {e}"
                            ),
                        )
                    });
                *active_key = Some((header, key));
            }
            &active_key.as_ref().expect("set above").1
        });
        let payload = format::encode_v2_payload(entry, key.is_some())?;
        let index = active.take_frame_index().unwrap_or_else(|e| {
            wal_fail_stop(
                WalFailStopTier::P,
                &format!("partition={partition}, sequence={}: {e}", entry.sequence),
            )
        });
        // The index is spent: a failure from here on is past the frame
        // (`TG-WAL-007` tier (B)).
        let frame = format::frame_v2(&payload, key.map(|key| (key, index))).unwrap_or_else(|e| {
            wal_fail_stop(
                WalFailStopTier::B,
                &format!(
                    "WAL frame seal failed: partition={partition}, sequence={}: {e}",
                    entry.sequence
                ),
            )
        });
        Ok((frame, index))
    }
}

// ---------------------------------------------------------------------------
// Fail-stop
// ---------------------------------------------------------------------------

/// Which unrecoverable WAL condition triggered a fail-stop.
///
/// The tier is what an operator — and recovery — reads differently; the
/// mechanism is identical for both, so the tier lives in the log and in the
/// classification only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalFailStopTier {
    /// A programming bug detected BEFORE the entry was framed, so no byte of it
    /// reached the segment: an append targeted a sealed segment (the WAL's own
    /// rotation bookkeeping is inconsistent), or it carried a frame kind this
    /// build has no writer for.
    P,
    /// A write, flush or fsync failed after the entry may already have been
    /// framed into the segment, so the frame's durability is unknown.
    B,
}

/// Tiers observed by [`wal_fail_stop`] under test builds, oldest first.
#[cfg(test)]
static FAIL_STOP_OBSERVATIONS: std::sync::Mutex<Vec<WalFailStopTier>> =
    std::sync::Mutex::new(Vec::new());

/// Serialises every test that asserts on a recorded fail-stop tier.
///
/// The observation log is append-only and process-global, so two fail-stop tests
/// running in parallel interleave their entries and make an index-based read
/// return the other test's tier. Holding this across the act keeps each scenario
/// reading only its own entry — which is what keeps tier P distinguishable from
/// tier B rather than merely "something stopped".
///
/// Async-aware because the scenarios it guards await on real WAL and store I/O
/// between snapshotting the log length and reading their own entry.
#[cfg(test)]
pub(crate) static FAIL_STOP_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Terminates the process on an unrecoverable WAL condition. Never returns.
///
/// `abort()` — not `panic!` — is the mechanism, because the workspace builds with
/// the unwind panic strategy (switching it is a protected performance decision).
/// An unwinding panic here would be contained by the calling tokio task and the
/// process would survive on a WAL already known to be broken, panicking again on
/// every later append to the same partition. `abort()` is independent of the
/// panic strategy; `std::process::exit` is rejected because it runs destructors
/// that can block on the very WAL that is failing, and returning an error is
/// exactly what this disposition exists to forbid.
///
/// Under `cfg(test)` it records the tier and panics instead, so a test can
/// observe which tier fired without the harness process being killed.
pub(crate) fn wal_fail_stop(tier: WalFailStopTier, ctx: &str) -> ! {
    tracing::error!(
        target: "topgun::wal",
        tier = ?tier,
        ctx,
        "WAL fail-stop: unrecoverable condition, terminating process"
    );

    #[cfg(test)]
    {
        FAIL_STOP_OBSERVATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(tier);
        panic!("WAL fail-stop at tier {tier:?}: {ctx}");
    }

    #[cfg(not(test))]
    std::process::abort();
}

#[async_trait]
impl Wal for WalWriter {
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
        #[cfg(test)]
        self.test_park_append(partition).await;

        let handle = self.handle(partition).await?;
        // Take ONLY the active-append lock. GC of sealed segments runs under a
        // distinct lock and never blocks this path; rotation briefly takes this
        // same lock, which is correct — an append and a rotate on the same
        // partition must serialize so a seal never freezes a half-written frame.
        let mut active = handle.active.lock().await;

        // A `finish` that wrote rollback hints may unlink this partition's
        // active segment, so nothing may be framed after it: the write is
        // refused before a byte or an index is spent (`TG-WAL-007` tier (A)).
        if self.appends_closed.load(Ordering::Acquire) {
            anyhow::bail!(
                "WAL append refused: the WAL is closed for shutdown; partition={partition}, \
                 sequence={}",
                entry.sequence
            );
        }

        // A sealed append target means the WAL's own rotation bookkeeping is
        // inconsistent — a programming bug that no runtime disposition can repair
        // and that a rollback would only paper over, leaving the process running
        // on a broken WAL. Checked structurally, under the guard the append
        // already holds, so no byte of this entry can have been framed yet.
        if active.is_sealed() {
            wal_fail_stop(
                WalFailStopTier::P,
                &format!(
                    "append targeted a sealed segment: partition={partition}, sequence={}, path={}",
                    entry.sequence,
                    active.path().display()
                ),
            );
        }

        // Encoded, sealed and indexed under this lock (`TG-ENC-005`).
        let (frame, index) = self.frame_under_lock(partition, &handle, &mut active, entry)?;
        #[cfg(test)]
        if let Some(header) = active.header() {
            self.test_observe(WalTestEvent::Framed {
                salt: header.salt,
                index,
                sequence: entry.sequence,
            });
        }
        #[cfg(not(test))]
        let _ = index;

        // Every residual failure below is tier (B) BY CONSTRUCTION: the pre-check
        // has already excluded (P), so no error content needs inspecting. A frame
        // may already be in the segment and its durability is unknown, so the
        // process stops and recovery re-derives state from durable frames — a
        // retried fsync can report success for bytes the OS has already dropped.
        let count = active
            .append_frame(&frame, entry.sequence)
            .await
            .unwrap_or_else(|e| {
                wal_fail_stop(
                    WalFailStopTier::B,
                    &format!(
                        "WAL frame append failed: partition={partition}, sequence={}: {e}",
                        entry.sequence
                    ),
                )
            });
        handle.note_append(frame.len());

        match self.policy {
            WalFsyncPolicy::PerOp => {
                active.sync_data().await.unwrap_or_else(|e| {
                    wal_fail_stop(
                        WalFailStopTier::B,
                        &format!(
                            "WAL fsync failed: partition={partition}, sequence={}: {e}",
                            entry.sequence
                        ),
                    )
                });
                #[cfg(test)]
                self.test_observe_fsync(&active);
            }
            WalFsyncPolicy::Batched => {
                #[cfg(test)]
                let held = self.test_hold_batched_fsync.load(Ordering::Acquire);
                #[cfg(not(test))]
                let held = false;
                // Flush immediately when the batch threshold is reached so
                // high-throughput bursts don't wait for the timer task.
                if count >= 100 && !held {
                    active.sync_data().await.unwrap_or_else(|e| {
                        wal_fail_stop(
                            WalFailStopTier::B,
                            &format!(
                                "WAL batched fsync failed: partition={partition}, sequence={}: {e}",
                                entry.sequence
                            ),
                        )
                    });
                    #[cfg(test)]
                    self.test_observe_fsync(&active);
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

    /// Records that `sequence` is durable in the store as the partition's
    /// watermark: a commit carrying that row returned `Ok`.
    ///
    /// The in-memory copy moves to `max(current, sequence)` — no I/O, no seal,
    /// no GC; sealing and collection belong to the collection tick, which
    /// reads the store's rows itself.
    ///
    /// # Errors
    ///
    /// A partition outside the WAL's partitions.
    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        Self::durable_slot(&self.durable, partition)?.fetch_max(sequence, Ordering::AcqRel);
        Ok(())
    }

    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
        let applied_seq = Self::durable_slot(&self.durable, partition)?.load(Ordering::Acquire);
        #[cfg(test)]
        let applied_seq = self
            .test_unapplied_floor_override
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&partition)
            .copied()
            .unwrap_or(applied_seq);
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
                // scan above and this read. Skipping the vanished file is correct,
                // not a lost frame — but the reason is the WATERMARK's rule, not
                // the segment's: GC reclaims only segments covered by a
                // prefix-complete, inclusive, cross-incarnation watermark, i.e. one
                // that stops below the first frame any incarnation left un-applied.
                // So an unlinked segment's frames are durable in the inner store,
                // and the `retain(|e| e.sequence > applied_seq)` below would drop
                // them regardless. Were the watermark merely the highest sequence
                // that happened to be applied, this skip WOULD lose frames.
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
            let (entries, _) = self.decode_segment_keyed(&path, &data, tolerate_tail)?;
            all.extend(entries);
        }

        // Frames are written in ascending sequence order within and across
        // segments, but sort defensively so the ordering contract holds even if a
        // future path interleaves.
        all.sort_by_key(|e| e.sequence);
        all.retain(|e| e.sequence > applied_seq);
        Ok(all)
    }

    /// Fsyncs the active segment of every open partition that holds
    /// un-fsynced frames.
    ///
    /// Only an active segment can hold one: a sealed segment was fsynced
    /// before it was frozen. Each segment is synced under its own lock, taken
    /// after the map lock is released, so an append to another partition is
    /// never held up. A failure does not end the pass — the other partitions
    /// are still worth making durable — and the first one is what is returned.
    async fn sync(&self) -> anyhow::Result<()> {
        let handles: Vec<Arc<PartitionHandle>> =
            { self.partitions.lock().await.values().cloned().collect() };
        let mut first_error = None;
        for handle in handles {
            let mut active = handle.active.lock().await;
            if active.unfsynced_count() == 0 {
                continue;
            }
            match active.sync_data().await {
                Ok(()) => {
                    #[cfg(test)]
                    self.test_observe_fsync(&active);
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn durable_applied(&self, partition: u32) -> Option<u64> {
        self.durable
            .get(partition as usize)
            .map(|slot| slot.load(Ordering::Acquire))
    }

    /// Ends the collection tick and, when `write_hints`, closes the WAL to
    /// appends, writes the rollback hints of a clean stop and unlinks every
    /// segment they cover.
    ///
    /// A hint is `partition-{p:03}.applied` holding the partition's durable
    /// watermark — never a higher sequence, so an older binary started on
    /// this directory replays exactly the frames the data directory lacks.
    /// One per partition that has a segment file or a non-zero watermark;
    /// each a synced temporary file renamed into place, the directory synced
    /// once after the last. The segments are then collected as by a tick
    /// (`TG-WAL-003`), active ones included, so a directory with nothing
    /// above the watermark is left with no segment an older binary could not
    /// read.
    async fn finish(&self, write_hints: bool) -> anyhow::Result<usize> {
        let collection = &self.collection;
        collection.closed.store(true, Ordering::Release);
        collection.wake.notify_one();
        // Waits for a running tick to end; every later tick sees the flag.
        let mut collected = collection.collected.lock().await;
        if !write_hints {
            return Ok(0);
        }
        self.close_to_appends().await?;
        let with_segments = Self::segment_bytes_by_partition(&self.wal_dir)?;
        let mut written = 0usize;
        for partition in 0..PARTITION_COUNT {
            let watermark = Self::durable_slot(&self.durable, partition)?.load(Ordering::Acquire);
            if watermark == 0 && !with_segments.contains_key(&partition) {
                continue;
            }
            let path = self.applied_path(partition);
            Self::write_applied_file(&path, watermark).map_err(|e| {
                anyhow::anyhow!("Cannot write rollback hint {}: {e}", path.display())
            })?;
            written += 1;
            #[cfg(test)]
            if written == 1 {
                test_abort_if_armed(TestAbortPoint::FinishInsideHints);
            }
        }
        if written > 0 {
            fsync_dir(&self.wal_dir)?;
        }
        self.collect_covered_at_finish(&mut collected).await;
        Ok(written)
    }
}

impl WalWriter {
    /// Closes the WAL to appends for a `finish` that writes rollback hints.
    ///
    /// Taken under the open mutex and every installed partition's active
    /// lock, so no append is between its closed check and its write, and no
    /// partition opens afterwards. An append that arrived after the write
    /// path's last WAL sync is then fsynced here: the segment it is in may be
    /// kept, and its write was acknowledged. A failed fsync fails the
    /// `finish`, which then writes no hint and unlinks nothing, and the WAL
    /// stays closed: the frame's durability is unknown, so the stop must not
    /// look clean to a rollback.
    async fn close_to_appends(&self) -> anyhow::Result<()> {
        let _open = self.open.lock().await;
        let handles: Vec<Arc<PartitionHandle>> =
            { self.partitions.lock().await.values().cloned().collect() };
        let mut actives = Vec::with_capacity(handles.len());
        for handle in &handles {
            actives.push(handle.active.lock().await);
        }
        self.appends_closed.store(true, Ordering::Release);
        for active in &mut actives {
            if active.unfsynced_count() > 0 {
                active.sync_data().await?;
                #[cfg(test)]
                self.test_observe_fsync(active);
            }
        }
        Ok(())
    }

    /// Unlinks, at a `finish` that wrote rollback hints, every segment of
    /// every partition whose frames are all at or below the partition's
    /// bound — the lower of the durable watermark and a fresh read of the
    /// store's — a segment with no frame included, active or not, and after
    /// `wal-collected` covering them is durable (`TG-WAL-003`). Nothing is
    /// unlinked when the store's watermarks cannot be read, the store gone
    /// included. Segments are read from the directory, so a partition this
    /// process never opened is collected too. A segment that cannot be read
    /// or unlinked stays, logged by name.
    async fn collect_covered_at_finish(&self, collected: &mut Box<[u64]>) {
        let Some(store) = self.store.upgrade() else {
            tracing::warn!(
                target: "topgun_server::storage::wal_watermark",
                "WAL segments kept at shutdown: the store is gone, its watermarks cannot be read"
            );
            return;
        };
        let from_store = Self::tick_watermarks(store.as_ref());
        drop(store);
        let Some(from_store) = from_store else {
            return;
        };
        let partitions = match WalRecovery::discover_partitions(&self.wal_dir) {
            Ok(partitions) => partitions,
            Err(e) => {
                tracing::warn!(error = %e, "WAL segments kept at shutdown: the WAL directory could not be read");
                return;
            }
        };
        let mut next: Box<[u64]> = collected.clone();
        let mut covered: Vec<PathBuf> = Vec::new();
        for partition in partitions {
            let index = partition as usize;
            let (Some(durable), Some(stored)) = (self.durable.get(index), from_store.get(index))
            else {
                continue;
            };
            let bound = durable.load(Ordering::Acquire).min(*stored);
            #[cfg(test)]
            let bound = self
                .test_tick_watermark_override
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&partition)
                .map_or(bound, |forced| bound.min(*forced));
            let segments = match Self::discover_partition_segments(&self.wal_dir, partition) {
                Ok(segments) => segments,
                Err(e) => {
                    tracing::warn!(partition, error = %e, "WAL segments kept at shutdown");
                    continue;
                }
            };
            let last_idx = segments.len().saturating_sub(1);
            for (idx, (_, path)) in segments.into_iter().enumerate() {
                let decoded = std::fs::read(&path)
                    .map_err(|e| anyhow::anyhow!("Cannot read WAL segment {}: {e}", path.display()))
                    .and_then(|data| self.decode_segment_keyed(&path, &data, idx == last_idx));
                let entries = match decoded {
                    Ok((entries, _)) => entries,
                    Err(e) => {
                        tracing::error!(segment = %path.display(), error = %e, "WAL segment kept at shutdown: it could not be read");
                        continue;
                    }
                };
                let max_seq = entries.iter().map(|e| e.sequence).max().unwrap_or(0);
                if max_seq <= bound {
                    next[index] = next[index].max(max_seq);
                    covered.push(path);
                }
            }
        }
        if covered.is_empty() {
            return;
        }
        if let Err(e) = write_wal_collected(&self.wal_dir, &next) {
            tracing::error!(
                error = %e,
                "WAL segments kept at shutdown: wal-collected could not be written"
            );
            return;
        }
        *collected = next;
        for path in &covered {
            if let Err(e) = self.unlink_segment(path).await {
                tracing::error!(
                    segment = %path.display(),
                    error = %e,
                    "WAL segment kept at shutdown: it could not be unlinked"
                );
            }
        }
        if let Err(e) = fsync_dir(&self.wal_dir) {
            tracing::error!(error = %e, "WAL shutdown could not fsync the WAL directory");
        }
        match Self::segment_bytes_by_partition(&self.wal_dir) {
            Ok(retained) => set_retained_gauges(&retained),
            Err(e) => tracing::warn!(error = %e, "WAL retained bytes not updated"),
        }
    }

    /// Seals the active segment and rotates a fresh one in, if the active
    /// segment actually holds frames.
    ///
    /// Sealing an empty active segment would leave a zero-length sealed file
    /// the next append could never reach and GC would have to special-case,
    /// so this is a no-op when the active segment is empty.
    async fn seal_and_rotate_if_needed(
        &self,
        partition: u32,
        handle: &Arc<PartitionHandle>,
    ) -> anyhow::Result<()> {
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
            #[cfg(test)]
            self.test_observe_fsync(&active);

            let next_first_seq = active.max_seq().saturating_add(1);
            let new_path = self
                .wal_dir
                .join(segment::format_segment_filename(partition, next_first_seq));
            // The new segment's header and directory entry are durable BEFORE
            // any append is acked to it (before this lock is released). A
            // failure after its rename leaves it behind with no frame: valid,
            // read as empty, replaced by a later creation of the same name.
            let new_active = self.create_active(new_path, partition, next_first_seq)?;
            #[cfg(test)]
            if self
                .test_fail_rotation_dir_fsync
                .swap(false, Ordering::AcqRel)
            {
                anyhow::bail!("injected: the WAL directory fsync after a rotation failed");
            }

            // Swap in the fresh active, take the old one out to freeze it. The
            // old segment's data was already fsynced above, so the freeze is
            // the infallible `into_sealed` — no fallible fsync runs AFTER the
            // swap, which guarantees the old active can never be dropped
            // (orphaned) by a failing fsync after it has been replaced.
            let old_active = std::mem::replace(&mut *active, new_active);
            let frozen = old_active.into_sealed();
            handle.note_rotation();
            // The sealed-set lock is taken only for the push, never held across
            // the append-lock release, so appenders are not serialized by GC.
            handle.sealed.lock().await.push(frozen);
            // Active-append lock released here.
        }
        Ok(())
    }

    /// How many sealed segments, from the front of `sealed`, `watermark`
    /// covers and `revalidated` confirms. A segment `watermark` covers but
    /// `revalidated` does not is retained, counted and logged.
    fn reclaimable_prefix(
        partition: u32,
        sealed: &[Segment],
        watermark: u64,
        revalidated: u64,
    ) -> usize {
        // Reclaimable sealed segments are a prefix of the ascending-ordered set
        // (oldest first), valid because sequences are monotonic and gap-free.
        // The prefix stops at the FIRST segment the re-read does not cover:
        // beyond it every segment is higher still, so nothing reclaimable is
        // lost by stopping, and GC resumes on the next call.
        let mut reclaim = 0usize;
        for segment in sealed.iter().take_while(|s| s.max_seq() <= watermark) {
            if let Err(err) =
                Self::revalidate_before_unlink(partition, segment.max_seq(), revalidated)
            {
                describe_wal_watermark_metrics();
                metrics::counter!(
                    WAL_GC_SKIPPED_COUNTER,
                    "reason" => GC_SKIP_REASON_WATERMARK
                )
                .increment(1);
                tracing::error!(
                    target: "topgun_server::storage::wal_watermark",
                    partition,
                    segment = %segment.path().display(),
                    "{err}"
                );
                break;
            }
            reclaim += 1;
        }
        reclaim
    }

    /// Whether the tick seals `partition`'s active segment at `now`: it holds
    /// frames, and either enough bytes were appended to it or its first frame
    /// is old enough. Bounds the size of a segment and the time a frame waits
    /// before collection can reach it, without a seal per commit.
    fn seal_due(partition: u32, handle: &PartitionHandle, now: std::time::Instant) -> bool {
        if handle.active_bytes.load(Ordering::Relaxed) >= SEAL_BYTES {
            return true;
        }
        let first = *handle
            .active_first_append
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        first.is_some_and(|first| now.saturating_duration_since(first) >= seal_age(partition))
    }

    /// The store's watermark of every WAL partition, read once for a tick, or
    /// `None` — logged and counted — when it cannot be read.
    fn tick_watermarks(store: &dyn MapDataStore) -> Option<Vec<u64>> {
        let error = match store.applied_watermarks() {
            Ok(Some(watermarks)) if watermarks.by_partition.len() == PARTITION_COUNT as usize => {
                return Some(watermarks.by_partition);
            }
            Ok(Some(watermarks)) => format!(
                "the store returned {} watermarks, one per WAL partition is {PARTITION_COUNT}",
                watermarks.by_partition.len()
            ),
            Ok(None) => "the store keeps no applied watermark".to_string(),
            Err(e) => format!("{e:#}"),
        };
        tracing::warn!(
            target: "topgun_server::storage::wal_watermark",
            error,
            "WAL collection skipped: the applied watermarks could not be read"
        );
        describe_wal_watermark_metrics();
        metrics::counter!(WAL_GC_SKIPPED_COUNTER, "reason" => GC_SKIP_REASON_READ_FAILED)
            .increment(1);
        None
    }

    /// Unlinks the `count` oldest sealed segments of each handle, then fsyncs
    /// the directory once. Called only once `wal-collected` covers them.
    ///
    /// A segment leaves the sealed set only once its file is gone, so the set,
    /// the disk and the retained-bytes gauge agree. A segment that cannot be
    /// unlinked ends its partition's unlinking — the segments after it stay
    /// too, so the set keeps a contiguous run of frames — and the other
    /// partitions are still collected. A later tick unlinks what is left: its
    /// frames are at or below `wal-collected`, and a start filters them.
    async fn unlink_collected(&self, reclaim: Vec<(&Arc<PartitionHandle>, usize)>) {
        let mut unlinked = 0usize;
        for (handle, count) in reclaim {
            let mut sealed = handle.sealed.lock().await;
            let mut removed = 0usize;
            for segment in sealed.iter().take(count) {
                if let Err(e) = self.unlink_segment(segment.path()).await {
                    tracing::error!(
                        segment = %segment.path().display(),
                        error = %e,
                        "WAL collection could not unlink a sealed segment"
                    );
                    break;
                }
                removed += 1;
                unlinked += 1;
                #[cfg(test)]
                if unlinked == 1 {
                    test_abort_if_armed(TestAbortPoint::CollectInsideUnlink);
                }
            }
            sealed.drain(..removed);
        }
        if unlinked > 0 {
            #[cfg(test)]
            test_abort_if_armed(TestAbortPoint::CollectAfterUnlink);
            if let Err(e) = fsync_dir(&self.wal_dir) {
                tracing::error!(error = %e, "WAL collection could not fsync the WAL directory");
            }
        }
    }

    /// Unlinks one sealed segment file.
    async fn unlink_segment(&self, path: &Path) -> std::io::Result<()> {
        #[cfg(test)]
        if self
            .test_refused_unlinks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(path)
        {
            return Err(std::io::Error::other("injected: the unlink is refused"));
        }
        tokio::fs::remove_file(path).await
    }

    /// One collection tick (TG-WAL-003). Returns whether the tick task should
    /// go on.
    ///
    /// In order: read the store's watermarks once; seal the active segments
    /// that are due; select, per partition, the prefix of sealed segments at
    /// or below both the durable watermark and the store's; write
    /// `wal-collected` covering them and only then unlink them; fsync the
    /// directory once; set the retained-bytes gauges.
    ///
    /// Nothing is sealed, written or unlinked after `finish`, or when the
    /// watermarks cannot be read — the next tick tries again. The task ends
    /// after `finish` and once the store is gone.
    async fn collect(&self) -> bool {
        let mut collected = self.collection.collected.lock().await;
        if self.collection.closed.load(Ordering::Acquire) {
            return false;
        }
        let Some(store) = self.store.upgrade() else {
            return false;
        };
        // Read before the store, so a commit that returns in between can
        // only raise the store's value above it: a store value below the
        // durable one then means the store lost a row it acknowledged, which
        // is worth the counter and the log line.
        let durable_now: Vec<u64> = self
            .durable
            .iter()
            .map(|w| w.load(Ordering::Acquire))
            .collect();
        let from_store = Self::tick_watermarks(store.as_ref());
        drop(store);
        let Some(from_store) = from_store else {
            return true;
        };

        let partitions = match Self::segment_bytes_by_partition(&self.wal_dir) {
            Ok(partitions) => partitions,
            Err(e) => {
                tracing::warn!(error = %e, "WAL collection skipped: the WAL directory could not be read");
                return true;
            }
        };
        let mut handles: Vec<(u32, Arc<PartitionHandle>)> = Vec::new();
        for &partition in partitions.keys() {
            match self.handle(partition).await {
                Ok(handle) => handles.push((partition, handle)),
                Err(e) => tracing::warn!(
                    partition,
                    error = %e,
                    "WAL collection skipped a partition: its segments could not be opened"
                ),
            }
        }

        let now = std::time::Instant::now();
        #[cfg(test)]
        let now = now
            + *self
                .test_clock_offset
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (partition, handle) in &handles {
            if Self::seal_due(*partition, handle, now) {
                if let Err(e) = self.seal_and_rotate_if_needed(*partition, handle).await {
                    tracing::error!(partition, error = %e, "WAL collection could not seal a segment");
                }
            }
        }

        // The prefix each partition may lose, and the new collected-up-to
        // values that cover it. Only this tick changes a sealed set, so the
        // prefix is still in place when it is unlinked below.
        let mut reclaim: Vec<(&Arc<PartitionHandle>, usize)> = Vec::new();
        let mut next: Box<[u64]> = collected.clone();
        for (partition, handle) in &handles {
            let index = *partition as usize;
            let limit = from_store[index];
            #[cfg(test)]
            let limit = self
                .test_tick_watermark_override
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(partition)
                .map_or(limit, |forced| limit.min(*forced));
            let sealed = handle.sealed.lock().await;
            let count = Self::reclaimable_prefix(*partition, &sealed, durable_now[index], limit);
            if count > 0 {
                next[index] = next[index].max(sealed[count - 1].max_seq());
                reclaim.push((handle, count));
            }
        }

        if !reclaim.is_empty() {
            // Durable before the first unlink: a start that finds a segment
            // gone must find the mark that covers it, so it can refuse a data
            // directory older than what was unlinked.
            if let Err(e) = write_wal_collected(&self.wal_dir, &next) {
                tracing::error!(
                    error = %e,
                    "WAL collection unlinked nothing: wal-collected could not be written"
                );
                return true;
            }
            *collected = next;
            #[cfg(test)]
            test_abort_if_armed(TestAbortPoint::CollectAfterMark);

            self.unlink_collected(reclaim).await;
        }

        match Self::segment_bytes_by_partition(&self.wal_dir) {
            Ok(retained) => set_retained_gauges(&retained),
            Err(e) => tracing::warn!(error = %e, "WAL retained bytes not updated"),
        }
        true
    }

    /// Highest sequence this WAL has ever durably observed across **all**
    /// partitions: the max over every applied watermark AND every log's max
    /// sequence. The watermarks are the store's rows as this WAL holds them —
    /// the sidecars were deleted at the open.
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
        // A store row survives collection of every segment it covers, and a
        // WAL cut at its last fsync under the batched policy can end below
        // it, so every row bounds the seed whether or not frames remain.
        let mut global = self
            .durable
            .iter()
            .map(|slot| slot.load(Ordering::Acquire))
            .max()
            .unwrap_or(0);
        for partition in WalRecovery::discover_partitions(&self.wal_dir)? {
            global = global.max(self.max_log_sequence(partition).await?);
        }
        Ok(global)
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
            let (entries, _) = self.decode_segment_keyed(&path, &data, idx == last_idx)?;
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

/// The most frames one replay batch of a store-mode WAL takes.
const REPLAY_BATCH_MAX_FRAMES: usize = 1000;
/// The most encoded value bytes one replay batch takes; the first frame of a
/// batch is admitted whole.
const REPLAY_BATCH_MAX_BYTES: usize = 8 * 1024 * 1024;

/// What replaying one frame writes to the store.
enum ReplayWrite {
    /// Stores `value` under the frame's key.
    Put {
        /// The record.
        value: RecordValue,
        /// Absolute millis since epoch (0 = no expiry).
        expiration_time: i64,
    },
    /// Removes the frame's key.
    Remove,
}

/// The values a replay batch has prepared and not yet committed, by
/// `(map, key)`; `None` is a key the batch removes.
#[derive(Default)]
struct ReplayOverlay {
    values: HashMap<(String, String), Option<RecordValue>>,
}

/// One frame of a replay batch, once prepared.
enum Prepared {
    /// The frame writes nothing: an LWW frame the gate discards.
    Nothing,
    /// A put, as the msgpack encoding `add` stores.
    Put {
        encoded: Vec<u8>,
        /// Absolute millis since epoch (0 = no expiry).
        expiration_time: i64,
    },
    /// A remove.
    Remove,
    /// Preparing the frame failed, or the store refused its write: it blocks
    /// the frontier and stays replayable.
    Failed,
}

/// Where the replay of one partition stands.
struct ReplayProgress {
    /// The contiguous-success frontier so far.
    frontier: u64,
    /// The first frame that was not stored, if any.
    first_failed: Option<u64>,
    /// The partition's row as the store holds it.
    row: u64,
    /// The store reported itself broken.
    broken: bool,
}

/// What the replay of one partition did.
struct PartitionReplay {
    /// The contiguous-success frontier: every frame at or below it is stored.
    frontier: u64,
    /// The first frame that was not stored, if any.
    first_failed: Option<u64>,
    /// The batches committed.
    batches: usize,
}

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
    /// Whether the `RecordValue::Lww` merge-idempotency gate in `prepare_entry` is
    /// active. Always `true` in production; the harness flips it off to reproduce
    /// the pre-`TG-WAL-006` blind clobber.
    replay_merge_gate: bool,
    /// Whether `run` re-replays each partition's oldest un-applied frame once more
    /// after the in-order pass — a harness seam that manufactures the "older frame
    /// re-applied over a newer durable value" condition, which generated ops
    /// cannot: in-order replay always ends on the newest frame. The seam exists to
    /// exercise the merge gate. `#[cfg(test)]`-only: it does not exist in a
    /// production build.
    #[cfg(test)]
    re_replay_oldest_frame: bool,
    /// How many frames the `re_replay_oldest_frame` seam actually put back through
    /// replay during `run`.
    ///
    /// Exists so a case can assert the seam FIRED, not only that its outcome was
    /// clean. Every assertion over the gate-on seam expects GREEN — and a seam that
    /// silently stopped firing is GREEN too, so without this counter the whole
    /// re-replay family would hollow out into "no re-replay happened → no clobber",
    /// which is the trivial configuration the seam was written to replace.
    #[cfg(test)]
    re_replayed_frames: std::sync::atomic::AtomicUsize,
    /// Replay batch limits used instead of the production ones, so a test can
    /// make small windows span several batches.
    #[cfg(test)]
    test_batch_limits: Option<(usize, usize)>,
}

impl WalRecovery {
    /// Creates a `WalRecovery` that will replay entries for the given set of
    /// partition IDs. Pass an empty slice to auto-discover partitions from the
    /// log files in `wal_dir`.
    pub fn new(wal: Arc<WalWriter>, partitions: Vec<u32>) -> Self {
        Self {
            wal,
            partitions,
            replay_merge_gate: true,
            #[cfg(test)]
            re_replay_oldest_frame: false,
            #[cfg(test)]
            re_replayed_frames: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            test_batch_limits: None,
        }
    }

    /// Test-only: replay batches of at most `frames` frames or `bytes` encoded
    /// value bytes instead of the production limits.
    #[cfg(test)]
    pub(crate) fn test_set_replay_batch_limits(&mut self, frames: usize, bytes: usize) {
        self.test_batch_limits = Some((frames.max(1), bytes));
    }

    /// Test-only: disable (or re-enable) the `RecordValue::Lww` merge gate in
    /// `prepare_entry`. Disabling it reproduces the pre-`TG-WAL-006` blind replay.
    #[cfg(test)]
    pub(crate) fn test_set_replay_merge_gate(&mut self, enabled: bool) {
        self.replay_merge_gate = enabled;
    }

    /// Test-only: after the in-order replay pass, re-replay each partition's oldest
    /// un-applied frame once more — the harness seam that reproduces a stale
    /// re-replay over a newer durable value.
    #[cfg(test)]
    pub(crate) fn test_set_re_replay_oldest_frame(&mut self, enabled: bool) {
        self.re_replay_oldest_frame = enabled;
    }

    /// Test-only: how many frames the re-replay seam actually replayed during `run`.
    ///
    /// `Relaxed` is sufficient on both ends: the seam increments from inside `run`
    /// and every reader observes it only after `run`'s future has completed, so the
    /// await point already carries the happens-before edge.
    #[cfg(test)]
    pub(crate) fn test_re_replayed_frames(&self) -> usize {
        self.re_replayed_frames
            .load(std::sync::atomic::Ordering::Relaxed)
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

    /// Replays ONE entry through the inner store, writing at once what
    /// [`Self::prepare_entry`] decides; the contract of each arm is stated
    /// there.
    ///
    /// `run` replays in batches through `prepare_entry`; this one-frame path
    /// is kept for the tests and the re-replay seam, which drive one frame.
    #[cfg(test)]
    async fn replay_entry(
        inner_store: &Arc<dyn MapDataStore>,
        entry: &WalEntry,
        now: i64,
        merge_gate: bool,
    ) -> anyhow::Result<()> {
        let overlay = ReplayOverlay::default();
        match Self::prepare_entry(inner_store, &overlay, entry, merge_gate).await? {
            None => Ok(()),
            Some(ReplayWrite::Put {
                value,
                expiration_time,
            }) => {
                inner_store
                    .add(&entry.map, &entry.key, &value, expiration_time, now)
                    .await
            }
            Some(ReplayWrite::Remove) => inner_store.remove(&entry.map, &entry.key, now).await,
        }
    }

    /// Replays all un-applied entries for every partition through `inner_store`.
    ///
    /// Returns an error (and the caller should exit non-zero) if mid-file
    /// corruption is detected — see `WalWriter::unapplied`. A truncated tail
    /// is tolerated with a WARN log.
    ///
    /// A partition is marked applied only up to its **contiguous-success
    /// frontier**: the highest sequence such that every un-applied entry at or
    /// below it replayed successfully. The frontier — never the highest sequence
    /// that happened to succeed — is what makes the persisted watermark
    /// prefix-complete, so a single failed replay can no longer license GC of its
    /// own frame. Frames above the frontier stay in the replay window and are
    /// re-applied on a later boot.
    ///
    /// That re-application is idempotent for `RecordValue::Lww` (TG-WAL-006):
    /// `prepare_entry` reads the current durable value and discards a frame whose HLC
    /// timestamp is strictly older, so a stale replayed frame can no longer clobber a
    /// newer durable value written outside the window (e.g. a frameless write-behind
    /// flush). Legacy frames keep always-merge replay.
    ///
    /// OR frames come in TWO kinds, and each is idempotent under a DIFFERENT
    /// argument — neither argument covers the other kind (`TG-OR-003`):
    ///
    /// - **Snapshot frames** — a `WalOp::Store` carrying `RecordValue::OrMap` or
    ///   `RecordValue::OrTombstones` — are idempotent by pre-existing
    ///   monotonicity: they are post-RMW full snapshots, monotone in `wal_seq`
    ///   order, so re-applying one cannot move the slot backwards. Not by this
    ///   merge, which is Lww-scoped.
    /// - **`WalOp::OrDelta` frames** are idempotent by SET-ALGEBRA FOLD
    ///   idempotency through the single live apply seam, `crdt::apply_or_delta`:
    ///   `Add` is a tombstone-guarded `retain` + `push`, `Remove` dedups the
    ///   tombstone, and `Prune` is pure tombstone-set subtraction. The property
    ///   holds for the WHOLE un-applied window replayed in sequence order from
    ///   the durable-store base, which is exactly what this pass does.
    ///   Monotonicity is NOT the argument here and cannot be offered as one: a
    ///   delta carries only its own mutation, so it is monotone in nothing.
    ///
    /// `WalOp::Remove` re-replay is idempotent under a DISTINCT invariant
    /// (`TG-WAL-009`) WITHOUT a gate: a Remove replays UNCONDITIONALLY, identical to
    /// the live remove path (blind delete). Because this pass is prefix-complete and
    /// strictly in-order, a stale Remove at sequence N is replayed only when N sits
    /// above the applied watermark, which forces every higher-sequence frame —
    /// including any strictly-newer re-creation — above the watermark too, so the
    /// re-creation replays AFTER the Remove and survives; and a re-creation that is
    /// durable-but-unframed necessarily sits below the watermark (older than the
    /// replayed Remove), so deleting it is live-correct. The out-of-order
    /// stale-Remove-over-newer-value state is unreachable here and is fabricated
    /// only by the harness `re_replay_oldest_frame` seam.
    ///
    /// A partition whose frontier stops below its highest enumerated frame raises
    /// the abandoned-write alarm here, at boot. Write-behind's watchdog cannot
    /// cover this: a partition that never receives a live write is never seeded
    /// and never sampled, so without the boot alarm its retained frames would be
    /// safe but invisible.
    ///
    /// # The store's watermarks
    ///
    /// `run` reads the store's watermarks once, before replaying anything, and
    /// replays only frames above them. A frame at or below its partition's
    /// watermark that reaches
    /// the replay is an error, never skipped and never replayed: the window
    /// from the watermark up, re-applied whole in sequence order onto the
    /// durable base, converges, while one `WalOp::OrDelta` frame re-applied
    /// below it can bring back a tag a later remove and prune took out. The
    /// window is replayed in batches (see `replay_partition`): each batch is
    /// one store commit holding its frames' writes and, when the frontier
    /// moved, the partition's row, so the row is never durable without the
    /// writes it covers; only once that commit returned is the WAL told it is
    /// applied.
    ///
    /// # Violation posture
    ///
    /// Recovery never refuses to boot on the state it observes — except a
    /// frame at or below the store's watermark. Every input here
    /// — segment bytes, frame contents, the store's replay verdict — is
    /// environmental or adversarial, so it is handled as a value, never as a
    /// panic: a hard stop would brick an already-deployed server on upgrade over
    /// a pre-existing on-disk WAL, converting a durability fix into an outage. A
    /// panic is reserved for an inconsistency reachable ONLY through this
    /// process's own in-memory ordering, and recovery observes none. Legacy WAL
    /// states — including `WalStorePayload::Legacy` frames and a sidecar written
    /// before the frontier rule existed — boot and replay unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if a WAL file contains mid-file CRC corruption, an
    /// unrecognised magic, or an unknown format version; also if the store's
    /// watermarks cannot be read or a frame at or below one of them reaches
    /// the replay. A failed replay of an individual entry is NOT
    /// an error: it blocks the frontier and is logged.
    #[allow(clippy::too_many_lines)]
    pub async fn run(&self, inner_store: Arc<dyn MapDataStore>) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let partitions = if self.partitions.is_empty() {
            Self::discover_partitions(&self.wal.wal_dir)?
        } else {
            self.partitions.clone()
        };

        let store_watermarks = Self::store_watermarks(&inner_store)?;
        let mut frames = 0usize;
        let mut batches = 0usize;

        for partition_id in partitions {
            let entries = self.wal.unapplied(partition_id).await?;

            if entries.is_empty() {
                continue;
            }

            let store_watermark = store_watermarks
                .get(partition_id as usize)
                .copied()
                .unwrap_or(0);

            // `entries` is strictly ascending by WAL sequence: `Wal::unapplied`
            // sorts defensively (see its impl), and `replay_partition` replays in
            // that order. Both the contiguous-frontier watermark logic AND the gate-free
            // Remove-idempotency non-reachability argument (TG-WAL-009's premise
            // that a stale Remove is replayed only when every strictly-newer frame
            // for the key is replayed AFTER it) rely on this ascending order
            // (TG-WAL-011).
            tracing::info!(
                partition = partition_id,
                count = entries.len(),
                "WAL recovery: replaying unapplied entries"
            );

            // The watermark stops at the last CONTIGUOUSLY replayed entry, not at
            // the highest one that happened to succeed: a failed entry blocks the
            // prefix exactly as it does on the live path. For [100 Ok, 101 Err,
            // 102 Ok] the frontier is 100, so 101 AND 102 stay replayable.
            let highest_enumerated = entries.iter().map(|e| e.sequence).max().unwrap_or(0);
            let now = current_millis();

            let replay = self
                .replay_partition(&inner_store, partition_id, &entries, store_watermark, now)
                .await?;
            let frontier = replay.frontier;
            let first_failed = replay.first_failed;
            frames += entries.len();
            batches += replay.batches;

            // Harness-only seam: re-replay the oldest un-applied frame once more,
            // AFTER the in-order pass has left the newest value durable. This
            // manufactures a stale re-replay — an older frame applied over a
            // newer durable value — a condition generated ops cannot manufacture
            // because in-order replay always ends on the newest frame. With the merge gate
            // on (production) the re-replayed older frame is discarded; with it off
            // it clobbers, which the `value_equality` oracle catches.
            #[cfg(test)]
            if self.re_replay_oldest_frame {
                if let Some(oldest) = entries.first() {
                    let _ =
                        Self::replay_entry(&inner_store, oldest, now, self.replay_merge_gate).await;
                    self.re_replayed_frames
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }

            if frontier < highest_enumerated {
                // An idle partition never receives a live write, so write-behind
                // never seeds it and its watchdog never samples it. Without this
                // boot alarm the retained frames would be safe but SILENT, and
                // silence is the failure class the alarm exists to remove.
                // Emitted on write-behind's watchdog target, with the same class
                // label, so an operator has ONE place to watch for a stalled
                // watermark regardless of which side observed it.
                tracing::error!(
                    target: "topgun_server::storage::wal_watermark",
                    partition = partition_id,
                    sequence = first_failed.unwrap_or(frontier.saturating_add(1)),
                    frontier,
                    highest_enumerated,
                    class = "AbandonedWrite/BootUnreplayed",
                    "WAL recovery could not replay every frame for this partition: the applied \
                     watermark stops at the contiguous-success frontier and the frames above it \
                     stay replayable. They resolve by replaying on a future restart once the store \
                     accepts them; WAL GC stays held back until they do."
                );
            }
        }

        tracing::info!(
            frames,
            batches,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "WAL recovery complete"
        );
        Ok(())
    }

    /// The store's applied watermarks, one per partition, read once.
    fn store_watermarks(inner_store: &Arc<dyn MapDataStore>) -> anyhow::Result<Vec<u64>> {
        let watermarks = inner_store
            .applied_watermarks()
            .map_err(|e| {
                anyhow::anyhow!(
                    "WAL recovery: cannot read the applied watermarks of the store: {e:#}"
                )
            })?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "WAL recovery: the WAL keeps its applied watermark in the store, but the \
                     store it replays into keeps none"
                )
            })?;
        Ok(watermarks.by_partition)
    }

    /// What replaying `entry` writes, reading the key's current value from
    /// `overlay` first and from `inner_store` when the overlay does not hold
    /// the key; `None` when the frame writes nothing.
    ///
    /// The one place a frame's replay is decided, so the gate and the fold
    /// exist once: a store-mode replay batch collects what this returns, and
    /// the one-frame `replay_entry` of the tests writes it at once.
    ///
    /// Replay goes straight to the store rather than through the CRDT service layer, so
    /// the merge that upholds re-replay idempotency lives HERE, at the recovery
    /// boundary, not in the store (`write_one` stays a CRDT-agnostic blind insert).
    ///
    /// Re-replay of a `RecordValue::Lww` frame is idempotent (TG-WAL-006): before
    /// applying a modern Lww frame this reads the current durable value and, if the
    /// incoming frame's HLC timestamp is strictly OLDER than the stored value's,
    /// discards it (last-write-wins by timestamp). A stale replayed frame therefore
    /// can no longer clobber a newer durable value written outside the replay window
    /// (e.g. a frameless write-behind flush). Ties and newer timestamps write
    /// through unchanged. The read-then-write is not atomic, which is safe because
    /// recovery runs before the listener accepts connections — there are no
    /// concurrent writers at boot.
    ///
    /// The gate is scoped to modern `WalStorePayload::Record(RecordValue::Lww)`
    /// frames only:
    /// - `WalStorePayload::Legacy` frames carry a SYNTHESIZED timestamp (zero-epoch
    ///   when the WAL recorded none) whose intent is always-merge, so they BYPASS
    ///   the gate and keep the pre-existing blind replay.
    /// - `RecordValue::OrMap`/`OrTombstones` frames carried by a `WalOp::Store`
    ///   BYPASS the gate: their CROSS-NODE convergence is set-union of
    ///   tags+tombstones, not a scalar timestamp max — replay itself is never a
    ///   union but an absolute-set REPLACE, since unioning a snapshot onto the
    ///   durable value would resurrect tombstoned tags. Such a frame is a post-RMW
    ///   COMPLETE snapshot of the key that is monotone in `wal_seq` order, so
    ///   in-sequence replay converges to the newest snapshot regardless of
    ///   re-replay. That argument
    ///   covers snapshot framing only — a `WalOp::OrDelta` frame is not a snapshot
    ///   and rests on the separate argument below. OR merge-idempotency as its own
    ///   property is owned by `TG-OR-003`, not this path.
    /// - A cross-kind case (stored value is not `Lww` while the incoming frame is)
    ///   cannot be compared by timestamp, so it also BYPASSES the gate and writes
    ///   through.
    ///
    /// Re-replay of a `WalOp::Remove` frame is idempotent WITHOUT a gate, under a
    /// DISTINCT invariant (`TG-WAL-009`, Remove-scoped — not the `TG-WAL-006` value
    /// claim above). A Remove replays UNCONDITIONALLY, identical to the live remove
    /// path (a blind delete with no timestamp compare): `entry.timestamp` is not
    /// consulted. This is single-algebra-correct — a gate the live path does not
    /// have would make `replay(Remove) != live(Remove)` and would itself lose acked
    /// deletes (a Remove whose sourced value-HLC is strictly older than the LWW
    /// survivor would be wrongly skipped). No gate is NEEDED either: under
    /// prefix-complete, in-order replay of the unapplied window, a stale Remove at
    /// sequence N is replayed only when N is above the watermark, in which case
    /// every higher-sequence frame — including any strictly-newer re-creation — is
    /// also above the watermark and replayed AFTER it, so the re-creation survives;
    /// and the only way a re-creation is durable-but-unframed is that its sequence
    /// is below the watermark, i.e. OLDER than the replayed Remove, so deleting it
    /// is live-correct. The `stale-Remove-over-newer-frameless-value` juxtaposition
    /// is unreachable except by re-applying an already-applied older frame — the
    /// harness `re_replay_oldest_frame` seam, never a production path (see
    /// INVARIANTS.md TG-WAL-009 for the full state-machine argument).
    ///
    /// A `WalOp::OrDelta` frame is FOLDED, not written through. The arm loads the
    /// key's CURRENT value, folds THIS ONE delta onto it via
    /// [`OrDeltaFold::fold`], and writes the result back, so the durable store —
    /// never a WAL-window checkpoint — is both the fold base and the accumulator,
    /// and each frame is applied in its own sequence position inside the single
    /// replay stream rather than batched per key. A store-mode replay batch reads
    /// that value from its own-batch overlay when an earlier frame of the batch
    /// wrote the key, which is the value the store holds once that frame's write
    /// is committed. Re-replay is a semantic no-op
    /// because the folded algebra IS the live one and its ops are idempotent set
    /// ops (retain, dedup push, subtract) — not because a delta is a snapshot. A
    /// delta carries only its own mutation, which is why it can never be skipped
    /// or dropped: a dropped remove resurrects an entry, and unlike a full-snapshot
    /// frame a delta is never repaired by a later frame.
    ///
    /// KNOWN BOUND on that arm, deferred rather than closed (TODO-627): the fold
    /// writes the key back with NO expiry, and "no expiry is what the live OR path
    /// produces" is a property of the CONFIGURATION, not of the OR policy —
    /// `ExpiryPolicy::NONE` resolves through `compute_expiration_time`, which falls
    /// back to `StorageConfig::default_ttl_millis`, a real evictable field that is
    /// `0` in every production wiring but is not required to be. Wire a map-level
    /// default TTL and the snapshot arm carries the expiry across recovery while the
    /// delta arm discards it, silently, with nothing detecting the divergence. Stated
    /// here because a doc-contract that asserted TTL-equivalence between the two arms
    /// would be asserting a property this code does not have.
    ///
    /// A `WalOp::Store` frame carrying an OR value stays a SUPPORTED input and is
    /// applied in its sequence position as an ABSOLUTE SET, never as a fold-base
    /// skip hint and never as a union — see the arm for why the snapshot's
    /// completeness is what licenses the replace.
    ///
    async fn prepare_entry(
        inner_store: &Arc<dyn MapDataStore>,
        overlay: &ReplayOverlay,
        entry: &WalEntry,
        merge_gate: bool,
    ) -> anyhow::Result<Option<ReplayWrite>> {
        match &entry.op {
            WalOp::Store {
                value,
                expiration_time,
            } => {
                // Reconstruct the exact RecordValue. Current frames carry the full
                // value (LWW or OR), so OR-Map adds/tombstones replay losslessly.
                // Legacy frames carried a bare Value and are replayed as LWW; an
                // absent WAL timestamp (legacy/malformed) becomes a zero-epoch
                // timestamp so the inner store treats the replay as always-merge.
                let record_value = match value {
                    WalStorePayload::Record(rv) => rv.clone(),
                    WalStorePayload::Legacy(v) => {
                        let ts = entry.timestamp.clone().unwrap_or_else(|| Timestamp {
                            millis: 0,
                            counter: 0,
                            node_id: String::new(),
                        });
                        RecordValue::Lww {
                            value: v.clone(),
                            timestamp: ts,
                        }
                    }
                };

                // Merge-idempotency gate (TG-WAL-006), scoped to modern Lww frames.
                // Only a `Record`-payload `Lww` frame carries a real value timestamp
                // to compare; if the current value is a strictly-newer Lww value,
                // this frame is stale (it was superseded by a write outside the
                // replay window) and must be discarded rather than clobber it.
                // Legacy frames, OR frames, and a non-Lww current value all fall
                // through to the blind replay unchanged (see the doc-contract on
                // `prepare_entry`). `merge_gate` is always `true` in production; the
                // harness flips it off to reproduce the pre-fix blind clobber.
                if merge_gate {
                    if let WalStorePayload::Record(RecordValue::Lww {
                        timestamp: incoming_ts,
                        ..
                    }) = value
                    {
                        if let Some(RecordValue::Lww {
                            timestamp: stored_ts,
                            ..
                        }) = overlay.load(inner_store, &entry.map, &entry.key).await?
                        {
                            if *incoming_ts < stored_ts {
                                return Ok(None);
                            }
                        }
                    }
                }

                // ABSOLUTE SET, in this frame's sequence position — for an OR
                // value as much as for an LWW one. The write REPLACES the key's
                // value with this frame's; it never unions the frame with the
                // value the store already holds, and it is never a "skip the
                // fold up to here" hint. A delta frame later in the window then
                // folds onto what this frame wrote.
                //
                // What licenses the replace is that a `WalOp::Store` OR frame is
                // a COMPLETE post-state snapshot of ONE key: it carries the live
                // set AND the tombstone set as of its own sequence, so every
                // effect at or below that sequence — removes included — is
                // already inside it. Completeness, not recency, is the load-
                // bearing condition: a partial or live-set-only OR snapshot
                // would make this replace UNSAFE, because the tombstones it
                // omitted would come back from the store's older value. That
                // completeness is TG-WAL-012, and it holds by the payload TYPE
                // — `WalEntry` is per-key and carries a whole `RecordValue`,
                // whose `OrMap` shape has no partial form to frame.
                //
                // A naive union/merge here would be a correctness trap in the
                // other direction: the store's pre-crash value may still hold a
                // tag live that this frame tombstoned, and unioning resurrects
                // it. Deletes do not self-heal.
                //
                // Ordering and window bounds are inherited, not re-derived here:
                // the APPLIED watermark is prefix-complete and never leads
                // durable state (TG-WAL-005 — the wal_seq-space tracker; NOT
                // TG-WB-001, which is the entry-space FLUSHED watermark and
                // explicitly independent of it), sequences are minted strictly
                // monotone per partition in that same wal_seq space at append
                // time (TG-WAL-010), and the window is replayed strictly
                // ascending in it (TG-WAL-011).
                Ok(Some(ReplayWrite::Put {
                    value: record_value,
                    expiration_time: expiration_time.unwrap_or(0),
                }))
            }
            WalOp::Remove => {
                // Replay a Remove UNCONDITIONALLY (TG-WAL-009) — identical to the
                // live remove path, which is a blind delete with no timestamp
                // compare. `entry.timestamp` is deliberately NOT consulted and
                // `merge_gate` is ignored for this arm: a gate the live path does not
                // have would make `replay(Remove) != live(Remove)` and would itself
                // lose acked deletes (the exact regression an HLC-sourced gate
                // produced). No gate is needed — under prefix-complete in-order
                // replay a stale Remove can never be juxtaposed over a strictly-newer
                // re-creation (see `run`'s doc-contract and INVARIANTS.md TG-WAL-009
                // for the state-machine argument). The harness reproduces the
                // synthetic out-of-order clobber via its `re_replay_oldest_frame`
                // seam, not through this arm.
                Ok(Some(ReplayWrite::Remove))
            }
            WalOp::OrDelta { delta } => {
                // One frame at a time: load the key's CURRENT value, fold this ONE
                // delta onto it, write it back. The durable store is the
                // accumulator, so nothing here collects a key's deltas out of the
                // replay stream — the next frame for this key reads what this one
                // wrote, exactly as the live path's read-modify-write does. In a
                // store-mode replay batch that read is served by the own-batch
                // overlay while the batch is not yet committed: the overlay holds
                // what this frame's write will have made the store hold.
                let base = overlay.load(inner_store, &entry.map, &entry.key).await?;
                let folded = <Self as OrDeltaFold>::fold(base, delta);

                // Expiration `0` ("no expiry"): `load` returns no TTL and `add`
                // requires one, and a delta frame carries no expiration field —
                // it records a mutation, not a whole slot. `0` matches what the
                // live OR write path produces today, so the fold introduces no
                // TTL drift. An OR path that starts writing TTLs would need the
                // frame to carry one; it is not a value this arm may invent.
                //
                // "Produces `0` today" is a property of the CONFIGURATION, not of
                // the OR policy, and nothing detects the divergence if that
                // default ever moves — the KNOWN BOUND clause in `prepare_entry`'s
                // doc-contract states it in full, with its owner.
                Ok(Some(ReplayWrite::Put {
                    value: folded,
                    expiration_time: 0,
                }))
            }
            // An explicit arm rather than a `_` catch-all: the exhaustiveness is
            // what forces a future framing to declare its replay semantics here
            // instead of inheriting someone else's. This variant is never
            // appended by production code, so reaching replay means a test built
            // a frame it has no way to apply.
            #[cfg(test)]
            WalOp::TestNonSubsuming => Err(anyhow::anyhow!(
                "partial-state WAL framing has no replay semantics: map={}, key={}, sequence={}",
                entry.map,
                entry.key,
                entry.sequence
            )),
        }
    }

    /// The batch limits of a store-mode replay.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn replay_batch_limits(&self) -> (usize, usize) {
        #[cfg(test)]
        if let Some(limits) = self.test_batch_limits {
            return limits;
        }
        (REPLAY_BATCH_MAX_FRAMES, REPLAY_BATCH_MAX_BYTES)
    }

    /// Prepares `entry` for a batch reading `overlay`, and encodes a put as
    /// the store's `add` would. Returns the frame's write and the value the
    /// overlay takes for its key once the frame is admitted (`Some(None)` for
    /// a remove, `None` when the key's value does not change).
    async fn prepare_frame(
        &self,
        inner_store: &Arc<dyn MapDataStore>,
        overlay: &ReplayOverlay,
        partition: u32,
        entry: &WalEntry,
    ) -> (Prepared, Option<Option<RecordValue>>) {
        let prepared =
            Self::prepare_entry(inner_store, overlay, entry, self.replay_merge_gate).await;
        let encoded = match prepared {
            Ok(Some(ReplayWrite::Put {
                value,
                expiration_time,
            })) => rmp_serde::to_vec_named(&value)
                .map(|encoded| {
                    (
                        Prepared::Put {
                            encoded,
                            expiration_time,
                        },
                        Some(Some(value)),
                    )
                })
                .map_err(anyhow::Error::from),
            Ok(Some(ReplayWrite::Remove)) => Ok((Prepared::Remove, Some(None))),
            Ok(None) => Ok((Prepared::Nothing, None)),
            Err(err) => Err(err),
        };
        encoded.unwrap_or_else(|err| {
            log_unreplayed(partition, entry, &err);
            (Prepared::Failed, None)
        })
    }

    /// Prepares `frames` again in sequence order over a fresh overlay, so each
    /// reads the store as the writes committed so far left it; a frame whose
    /// write the store refused stays failed.
    async fn reprepare(
        &self,
        inner_store: &Arc<dyn MapDataStore>,
        partition: u32,
        frames: &[WalEntry],
        refused: &[bool],
    ) -> Vec<Prepared> {
        let mut overlay = ReplayOverlay::default();
        let mut prepared = Vec::with_capacity(frames.len());
        for (entry, refused) in frames.iter().zip(refused) {
            if *refused {
                prepared.push(Prepared::Failed);
                continue;
            }
            let (frame, value) = self
                .prepare_frame(inner_store, &overlay, partition, entry)
                .await;
            if let Some(value) = value {
                overlay.note(entry, value);
            }
            prepared.push(frame);
        }
        prepared
    }

    /// Replays the un-applied `entries` of a store-mode partition in batches.
    ///
    /// A batch takes frames in sequence order (TG-WAL-011) up to
    /// [`REPLAY_BATCH_MAX_FRAMES`] or [`REPLAY_BATCH_MAX_BYTES`] of encoded
    /// values, its first frame whole. Each frame is prepared against the
    /// batch's own-batch overlay, so it reads what an earlier frame of the
    /// batch wrote exactly as per-frame replay reads it from the store — the
    /// LWW gate (TG-WAL-006), the OR fold (TG-OR-003) and an unconditional
    /// Remove (TG-WAL-009) see the same values. The batch is one store commit
    /// holding its writes and, when the contiguous-success frontier moved
    /// above the row the store holds, the partition's row: the row is never
    /// durable without the writes it covers. Only once that commit returned is
    /// the WAL told the frontier is applied.
    ///
    /// Every frame is checked against `watermark`, the store's row read when
    /// `run` began: a frame at or below it refuses the start, naming it, and is
    /// never skipped. A frame that fails to prepare writes nothing and stops
    /// the frontier below it; later frames are still replayed, as per-frame
    /// replay does. A refused commit is handled by `commit_batch`; only a store
    /// that reports itself broken ends the partition's replay early.
    async fn replay_partition(
        &self,
        inner_store: &Arc<dyn MapDataStore>,
        partition: u32,
        entries: &[WalEntry],
        watermark: u64,
        now: i64,
    ) -> anyhow::Result<PartitionReplay> {
        let (max_frames, max_bytes) = self.replay_batch_limits();
        let mut progress = ReplayProgress {
            frontier: 0,
            first_failed: None,
            row: watermark,
            broken: false,
        };
        let mut batches = 0;
        let mut next = 0;
        while next < entries.len() && !progress.broken {
            let first = next;
            let mut overlay = ReplayOverlay::default();
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            while let Some(entry) = entries.get(next) {
                if batch.len() == max_frames {
                    break;
                }
                if entry.sequence <= watermark {
                    anyhow::bail!(
                        "WAL recovery: refusing to start: partition {partition} holds frame {} \
                         at or below its applied watermark {watermark}; a frame below the \
                         watermark is never replayed",
                        entry.sequence
                    );
                }
                let (frame, value) = self
                    .prepare_frame(inner_store, &overlay, partition, entry)
                    .await;
                let len = frame.encoded_len();
                if !batch.is_empty() && bytes.saturating_add(len) > max_bytes {
                    break;
                }
                bytes = bytes.saturating_add(len);
                if let Some(value) = value {
                    overlay.note(entry, value);
                }
                batch.push(frame);
                next += 1;
            }
            batches += 1;
            self.commit_batch(
                inner_store,
                partition,
                &entries[first..next],
                batch,
                &mut progress,
                now,
            )
            .await;
        }
        Ok(PartitionReplay {
            frontier: progress.frontier,
            first_failed: progress.first_failed,
            batches,
        })
    }

    /// Commits one replay batch, its row inside.
    ///
    /// A commit refused with [`RecordRefused`] leaves out the refused writes —
    /// their frames block the frontier — and commits the rest again; a commit
    /// refused otherwise is halved, each half committed in order, down to
    /// single frames, at most `2n − 1` commits for a batch of `n`. A frame
    /// refused alone blocks the frontier and the frames after it are still
    /// committed, as per-frame replay does; there is no hold to protect at
    /// start, so no refusal pattern ends the partition. Before a part is
    /// committed again its frames are prepared again from the store, which by
    /// then holds every write of this batch committed so far, so a refused
    /// write is never read by a later frame. A store that reports itself
    /// broken (`TG-WB-010`) ends the partition's replay: nothing more is
    /// committed for it.
    async fn commit_batch(
        &self,
        inner_store: &Arc<dyn MapDataStore>,
        partition: u32,
        frames: &[WalEntry],
        prepared: Vec<Prepared>,
        progress: &mut ReplayProgress,
        now: i64,
    ) {
        let mut refused = vec![false; frames.len()];
        // Parts still to commit, the next one last.
        let mut parts = vec![(0, frames.len())];
        let mut first_attempt = Some(prepared);
        while let Some((lo, hi)) = parts.pop() {
            let mut part = match first_attempt.take() {
                Some(prepared) => prepared,
                None => {
                    self.reprepare(inner_store, partition, &frames[lo..hi], &refused[lo..hi])
                        .await
                }
            };
            loop {
                let (writes, origin) = batch_writes(&frames[lo..hi], &part, now);
                let frontier = progress.frontier_after(&frames[lo..hi], &part);
                let applied = if frontier > progress.row {
                    vec![(partition, frontier)]
                } else {
                    Vec::new()
                };
                if writes.is_empty() && applied.is_empty() {
                    progress.settle(&frames[lo..hi], &part);
                    break;
                }
                let commit = crate::storage::map_data_store::FlushCommit {
                    writes,
                    applied: applied.clone(),
                };
                let err = match inner_store.commit(&commit).await {
                    Ok(()) => {
                        progress.settle(&frames[lo..hi], &part);
                        if let Some(&(_, row)) = applied.first() {
                            progress.row = row;
                            if let Err(err) = self.wal.mark_applied(partition, row).await {
                                tracing::warn!(
                                    partition,
                                    frontier = row,
                                    error = %err,
                                    "WAL recovery: failed to mark entries applied; \
                                     next restart will re-replay (safe but redundant)"
                                );
                            }
                        }
                        break;
                    }
                    Err(err) => err,
                };
                if let crate::storage::map_data_store::StoreErrorClass::StoreBroken(cause) =
                    inner_store.classify_error(&err)
                {
                    tracing::error!(
                        partition,
                        cause = cause.as_str(),
                        error = %err,
                        "WAL recovery: the durable store is broken; the replay of this \
                         partition stops and its frames stay replayable"
                    );
                    progress.broken = true;
                    return;
                }
                let mut named: Vec<usize> = err
                    .chain()
                    .find_map(|e| e.downcast_ref::<crate::storage::map_data_store::RecordRefused>())
                    .map(|r| {
                        r.indices
                            .iter()
                            .filter_map(|w| origin.get(*w).copied())
                            .collect()
                    })
                    .unwrap_or_default();
                // Only the first refused write of a key is taken as refused: a
                // later write of that key was prepared from the refused one,
                // so it is prepared again without it and committed again.
                named.sort_unstable();
                let mut keys = HashSet::new();
                named.retain(|i| keys.insert((&frames[lo + i].map, &frames[lo + i].key)));
                if !named.is_empty() {
                    for i in named {
                        let entry = &frames[lo + i];
                        log_unreplayed(partition, entry, &err);
                        refused[lo + i] = true;
                    }
                    part = self
                        .reprepare(inner_store, partition, &frames[lo..hi], &refused[lo..hi])
                        .await;
                    continue;
                }
                if hi - lo == 1 {
                    let entry = &frames[lo];
                    log_unreplayed(partition, entry, &err);
                    refused[lo] = true;
                    progress.settle(&frames[lo..hi], &[Prepared::Failed]);
                    break;
                }
                let mid = lo + (hi - lo) / 2;
                parts.push((mid, hi));
                parts.push((lo, mid));
                break;
            }
        }
    }
}

/// Logs a frame whose replay failed: it blocks the frontier and stays
/// replayable.
fn log_unreplayed(partition: u32, entry: &WalEntry, err: &anyhow::Error) {
    tracing::warn!(
        partition,
        map = %entry.map,
        key = %entry.key,
        seq = entry.sequence,
        error = %err,
        "WAL recovery: replay failed for entry; \
         watermark stops below it and it stays replayable"
    );
}

/// The store writes of a prepared part of a replay batch, and for each write
/// the index of its frame in the part.
fn batch_writes<'a>(
    frames: &'a [WalEntry],
    prepared: &[Prepared],
    now: i64,
) -> (
    Vec<crate::storage::map_data_store::StoreWrite<'a>>,
    Vec<usize>,
) {
    use crate::storage::map_data_store::{PutValue, StoreWrite};
    let mut writes = Vec::new();
    let mut origin = Vec::new();
    for (i, (frame, entry)) in prepared.iter().zip(frames).enumerate() {
        let write = match frame {
            Prepared::Put {
                encoded,
                expiration_time,
            } => StoreWrite::Put {
                map: &entry.map,
                key: &entry.key,
                value: PutValue::Encoded(encoded.clone()),
                expiration_time: *expiration_time,
                store_time: now,
            },
            Prepared::Remove => StoreWrite::Remove {
                map: &entry.map,
                key: &entry.key,
                store_time: now,
            },
            Prepared::Nothing | Prepared::Failed => continue,
        };
        origin.push(i);
        writes.push(write);
    }
    (writes, origin)
}

impl ReplayOverlay {
    /// The current value of `(map, key)`: what the batch has prepared for it,
    /// else what the store holds.
    async fn load(
        &self,
        inner_store: &Arc<dyn MapDataStore>,
        map: &str,
        key: &str,
    ) -> anyhow::Result<Option<RecordValue>> {
        match self.values.get(&(map.to_string(), key.to_string())) {
            Some(value) => Ok(value.clone()),
            None => inner_store.load(map, key).await,
        }
    }

    /// Records the value `entry`'s key holds once the batch is committed.
    fn note(&mut self, entry: &WalEntry, value: Option<RecordValue>) {
        self.values
            .insert((entry.map.clone(), entry.key.clone()), value);
    }
}

impl Prepared {
    /// The bytes this frame adds to its batch.
    fn encoded_len(&self) -> usize {
        match self {
            Self::Put { encoded, .. } => encoded.len(),
            Self::Remove | Self::Nothing | Self::Failed => 0,
        }
    }
}

impl ReplayProgress {
    /// The frontier once `prepared` — the frames `frames` — is committed.
    fn frontier_after(&self, frames: &[WalEntry], prepared: &[Prepared]) -> u64 {
        let mut frontier = self.frontier;
        if self.first_failed.is_none() {
            for (entry, frame) in frames.iter().zip(prepared) {
                if matches!(frame, Prepared::Failed) {
                    break;
                }
                frontier = entry.sequence;
            }
        }
        frontier
    }

    /// Takes in the outcome of `frames`, committed or failed.
    fn settle(&mut self, frames: &[WalEntry], prepared: &[Prepared]) {
        // Only a success that is still contiguous with the prefix may advance
        // the frontier. Once an entry has failed, later successes are kept —
        // re-running them next boot is free — but they must NOT license GC of
        // the frame that failed.
        //
        // "Free" rests on a different argument per frame kind: Lww on the
        // read-compare in `prepare_entry`; a `WalOp::Store` OR snapshot on
        // monotonicity; and a `WalOp::OrDelta` on set-algebra fold idempotency
        // through `crdt::apply_or_delta`, over the whole un-applied window
        // replayed in sequence order from the durable-store base. Monotonicity
        // does not cover the delta kind.
        for (entry, frame) in frames.iter().zip(prepared) {
            if matches!(frame, Prepared::Failed) {
                self.first_failed.get_or_insert(entry.sequence);
            } else if self.first_failed.is_none() {
                self.frontier = entry.sequence;
            }
        }
    }
}

/// The recovery fold: ONE delta, folded onto the key's CURRENT durable value.
///
/// `WalRecovery` is the implementor because it is already the object that owns
/// replay — a separate fold type would be a second seam over the same algebra.
/// The receiver is dropped because the fold is a pure function of
/// `(base, delta)`: `prepare_entry` is an associated function reached as
/// `WalRecovery::prepare_entry(inner_store, overlay, entry, merge_gate)` and holds no
/// `WalRecovery` value to call a method on.
///
/// The fold owns no algebra of its own. It normalizes the base through the live
/// `crdt::normalize_to_or_map` and applies through the live
/// `crdt::apply_or_delta` — the same two functions the runtime OR write path
/// calls, in the same order (TG-OR-003). A second hand-written copy of either
/// would be free to drift from what the write path actually did, and on this
/// path drift is a silently lost mutation rather than a visible error.
impl OrDeltaFold for WalRecovery {
    /// One frame, one call: the caller passes the key's current value as `base`
    /// and writes the result back, so the DURABLE STORE is the accumulator. In a
    /// store-mode replay batch that value comes from the batch's own-batch
    /// overlay when an earlier frame of the batch wrote the key — the value the
    /// store holds once the batch is committed. The parameter is a single
    /// delta, not a window, so a key's frames are still folded one at a time in
    /// sequence order, never collected out of the replay stream.
    fn fold(base: Option<RecordValue>, delta: &OrDelta) -> RecordValue {
        // An absent record is a legitimate first write, not an error: the empty
        // OR-Map here is byte-identical to the `init` value the live OR_ADD /
        // OR_REMOVE paths hand `update_in_place` for an absent slot.
        let mut value = base.unwrap_or(RecordValue::OrMap {
            records: Vec::new(),
            tombstones: Vec::new(),
        });

        // MUST precede the apply. `apply_or_delta` is a silent no-op on a value
        // that is not `RecordValue::OrMap` — no `Err`, no flag — so a legacy
        // `OrTombstones` blob or a cross-kind durable value would swallow this
        // delta with nothing for an oracle to observe. Upgrading the storage
        // shape is a concern of the slot, and this is the slot's one normalizer.
        crate::service::domain::crdt::normalize_to_or_map(&mut value);

        // The fold borrows and the apply consumes, so each delta is cloned once.
        // That is deliberate: the by-value apply is what keeps the LIVE path free
        // of a value-sized copy per accepted add, and this cost sits on the cold
        // boot path, bounded by the un-applied replay window.
        //
        // The returned flags are deliberately dropped. They exist for the live
        // path's tombstone-bytes gauge, which recovery does not maintain
        // incrementally — the boot-time `reconcile_tombstone_bytes` recompute
        // runs after replay and is the gauge's source of truth (TG-OR-004).
        crate::service::domain::crdt::apply_or_delta(delta.clone(), &mut value);

        value
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
// Test seams
// ===========================================================================

/// Accessors that open private WAL state to tests.
///
/// They exist because the state below is otherwise unreachable — `PartitionHandle`
/// and the pre-unlink re-validation are private, and sibling test modules
/// cannot see them at all.
#[cfg(test)]
impl WalWriter {
    /// Seals the partition's active segment in place.
    ///
    /// Seal/rotate always installs a FRESH active segment under the same lock, so
    /// a sealed active segment is not reachable through any production path. This
    /// is the only way to construct the state the append pre-check guards.
    pub(crate) async fn test_seal_active_segment(&self, partition: u32) -> anyhow::Result<()> {
        let handle = self.handle(partition).await?;
        let mut active = handle.active.lock().await;
        // `into_sealed` consumes the segment, so swap a placeholder in to take
        // ownership without ever leaving the guard holding an invalid value.
        let placeholder = Segment::sealed_existing(
            active.path().to_path_buf(),
            active.first_seq(),
            active.max_seq(),
        );
        let current = std::mem::replace(&mut *active, placeholder);
        *active = current.into_sealed();
        Ok(())
    }

    /// Runs the pre-unlink re-validation directly, so its typed `Err` can be
    /// asserted without driving a whole GC pass.
    pub(crate) fn test_revalidate_before_unlink(
        partition: u32,
        segment_max_seq: u64,
        revalidated: u64,
    ) -> Result<(), WalGcError> {
        Self::revalidate_before_unlink(partition, segment_max_seq, revalidated)
    }

    /// Fail-stop tiers recorded in this process, oldest first.
    pub(crate) fn test_fail_stop_observations() -> Vec<WalFailStopTier> {
        FAIL_STOP_OBSERVATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Seams of the collection tick and the fsyncs, for their tests.
#[cfg(test)]
impl WalWriter {
    /// Runs one collection tick now, as the timer would.
    pub(crate) async fn test_collect_now(&self) {
        self.collect().await;
    }

    /// Advances the clock the collection tick reads a segment's age by.
    pub(crate) fn test_advance_clock(&self, by: std::time::Duration) {
        *self
            .test_clock_offset
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += by;
    }

    /// Makes the collection tick use `value` for `partition` instead of the
    /// store's watermark when it is lower; `None` restores the store's.
    pub(crate) fn test_force_tick_watermark(&self, partition: u32, value: Option<u64>) {
        let mut overrides = self
            .test_tick_watermark_override
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match value {
            Some(value) => overrides.insert(partition, value),
            None => overrides.remove(&partition),
        };
    }

    /// Makes `unapplied(partition)` filter by `value` instead of the durable
    /// watermark; `None` restores the durable watermark.
    pub(crate) fn test_force_unapplied_floor(&self, partition: u32, value: Option<u64>) {
        let mut overrides = self
            .test_unapplied_floor_override
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match value {
            Some(value) => overrides.insert(partition, value),
            None => overrides.remove(&partition),
        };
    }

    /// Wakes the collection tick task now, without running the tick body in
    /// the caller.
    pub(crate) fn test_wake_collection_tick(&self) {
        self.collection.wake.notify_one();
    }

    /// Whether the collection tick task is still running.
    pub(crate) fn test_collection_tick_running(&self) -> bool {
        self.collection
            .task
            .get()
            .is_some_and(|task| !task.is_finished())
    }

    /// Installs the fsync observer, or removes it with `None`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn test_set_fsync_observer(
        &self,
        observer: Option<Arc<dyn Fn(&Path, u64) + Send + Sync>>,
    ) {
        *self
            .test_fsync_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = observer;
    }

    /// Hands the observer the length of the active segment just fsynced,
    /// read while the caller still holds the segment's lock: no frame can be
    /// appended between the fsync and the length.
    fn test_observe_fsync(&self, active: &Segment) {
        let observer = self
            .test_fsync_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            let length = std::fs::metadata(active.path()).map_or(0, |m| m.len());
            observer(active.path(), length);
        }
    }

    /// Holds every fsync of the batched policy while `hold` is set.
    pub(crate) fn test_hold_batched_fsync(&self, hold: bool) {
        self.test_hold_batched_fsync.store(hold, Ordering::Release);
    }

    /// Makes the collection tick fail to unlink the sealed segment at
    /// `path` while `refuse` is set.
    pub(crate) fn test_refuse_unlink(&self, path: &Path, refuse: bool) {
        let mut refused = self
            .test_refused_unlinks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if refuse {
            refused.insert(path.to_path_buf());
        } else {
            refused.remove(path);
        }
    }

    /// Installs the observer of created segments and framed frames, or
    /// removes it with `None`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn test_set_observer(
        &self,
        observer: Option<Arc<dyn Fn(WalTestEvent) + Send + Sync>>,
    ) {
        *self
            .test_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = observer;
    }

    fn test_observe(&self, event: WalTestEvent) {
        let observer = self
            .test_observer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    /// Makes every open that misses the installed map wait on `barrier`
    /// before it takes the open mutex; `None` removes it.
    pub(crate) fn test_set_open_barrier(&self, barrier: Option<Arc<tokio::sync::Barrier>>) {
        *self
            .test_open_barrier
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = barrier;
    }

    async fn test_wait_open_barrier(&self) {
        let barrier = self
            .test_open_barrier
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
    }

    /// Makes the next rotation fail after its new segment was created, as a
    /// failed directory fsync would.
    pub(crate) fn test_fail_next_rotation_dir_fsync(&self) {
        self.test_fail_rotation_dir_fsync
            .store(true, Ordering::Release);
    }

    /// Parks the next append to `partition` at the entry of `append`:
    /// `reached` is notified when it arrives, and it goes on once `release`
    /// is notified.
    pub(crate) fn test_park_next_append(
        &self,
        partition: u32,
        reached: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self
            .test_append_park
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(TestAppendPark {
            partition,
            reached,
            release,
        });
    }

    async fn test_park_append(&self, partition: u32) {
        let park = {
            let mut slot = self
                .test_append_park
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.as_ref().is_some_and(|p| p.partition == partition) {
                slot.take()
            } else {
                None
            }
        };
        if let Some(park) = park {
            park.reached.notify_one();
            park.release.notified().await;
        }
    }

    /// The paths of the sealed segments the WAL holds for `partition`,
    /// oldest first.
    pub(crate) async fn test_sealed_paths(&self, partition: u32) -> Vec<PathBuf> {
        let handle = self.handle(partition).await.expect("partition handle");
        let sealed = handle.sealed.lock().await;
        sealed.iter().map(|s| s.path().to_path_buf()).collect()
    }
}

/// What the test observer of a [`WalWriter`] is told.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WalTestEvent {
    /// A segment file was created (renamed into place) with `salt`.
    Created {
        partition: u32,
        path: PathBuf,
        salt: [u8; 32],
    },
    /// A frame was framed under the active lock as frame `index` of the
    /// segment whose header carries `salt`.
    Framed {
        salt: [u8; 32],
        index: u64,
        sequence: u64,
    },
}

/// An append parked at the entry of `append`.
#[cfg(test)]
pub(crate) struct TestAppendPark {
    partition: u32,
    reached: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

/// The environment variable that names the [`TestAbortPoint`] a child
/// process of the store-mode tests stops at.
///
/// An environment variable rather than a field: the open steps run inside
/// the constructor, before any instance exists to arm.
#[cfg(test)]
pub(crate) const TEST_ABORT_AT_ENV: &str = "TOPGUN_TEST_WAL_ABORT_AT";

/// A crash point of the store mode a test process can stop at, by
/// `std::process::abort`, to model `kill -9` there. Each fires only on the
/// path it names: a collection point only in a tick that unlinks a segment.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestAbortPoint {
    /// A collecting tick: `wal-collected` is durable, no segment unlinked yet.
    CollectAfterMark,
    /// A collecting tick: the first segment unlinked, the others not.
    CollectInsideUnlink,
    /// A collecting tick: every segment unlinked, the directory not yet
    /// fsynced.
    CollectAfterUnlink,
    /// The open: the first `.applied` file deleted, the others not.
    OpenInsideSidecarDelete,
    /// `finish`: the first rollback hint renamed into place, the others not.
    FinishInsideHints,
    /// A segment's creation: its temporary file written and synced, not yet
    /// renamed into place.
    CreateBeforeRename,
}

#[cfg(test)]
impl TestAbortPoint {
    /// The value of [`TEST_ABORT_AT_ENV`] that arms this point.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::CollectAfterMark => "collect-after-mark",
            Self::CollectInsideUnlink => "collect-inside-unlink",
            Self::CollectAfterUnlink => "collect-after-unlink",
            Self::OpenInsideSidecarDelete => "open-inside-sidecar-delete",
            Self::FinishInsideHints => "finish-inside-hints",
            Self::CreateBeforeRename => "create-before-rename",
        }
    }
}

/// Stops the process when [`TEST_ABORT_AT_ENV`] names `point`; does nothing
/// otherwise.
#[cfg(test)]
pub(crate) fn test_abort_if_armed(point: TestAbortPoint) {
    if std::env::var(TEST_ABORT_AT_ENV).is_ok_and(|armed| armed == point.name()) {
        std::process::abort();
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::map_data_store::{FlushCommit, WatermarkKeeping};
    use std::sync::Arc;
    use topgun_core::types::Value as TgValue;

    // -----------------------------------------------------------------------
    // Doc-contract assertions
    //
    // The OR contracts in this file are load-bearing rather than decorative:
    // `WalRecovery::run`'s idempotency paragraph is the text the
    // contiguous-success frontier cites to justify keeping post-failure
    // successes and re-replaying the window on the next boot. A contract that
    // under-describes a frame kind is therefore a false invariant in code, not
    // stale prose, so these assertions read this file's own source and fail the
    // build when a required clause goes missing or a withdrawn one returns.
    //
    // Every assertion runs on a WHITESPACE-NORMALIZED view. A single-line
    // search is NOT equivalent: several of the phrases below are line-wrapped
    // inside comments, so an un-normalized absence check would pass without
    // anyone having touched the contract it claims to police.
    // -----------------------------------------------------------------------

    /// This file's own source, truncated at the inline test module.
    ///
    /// Truncating is what keeps the assertions non-vacuous: the phrases they
    /// search for are spelled out as literals in the tests themselves, so a
    /// whole-file view would match a test's own argument instead of the
    /// production contract it is meant to check.
    fn production_source() -> &'static str {
        const SOURCE: &str = include_str!("mod.rs");
        // The escaped newline in this literal is not a newline in this file, so
        // the search cannot land on this line instead of the real marker.
        let marker = "#[cfg(test)]\nmod tests {";
        let end = SOURCE
            .find(marker)
            .expect("this file carries its tests in an inline `mod tests`");
        &SOURCE[..end]
    }

    /// Strips comment markers and indentation and collapses every run of
    /// whitespace to one space, so a phrase assertion can be neither satisfied
    /// nor defeated by where a line happens to wrap.
    fn normalized(src: &str) -> String {
        src.lines()
            .map(str::trim_start)
            .map(|l| {
                l.trim_start_matches("//!")
                    .trim_start_matches("///")
                    .trim_start_matches("//")
            })
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The source from `start` up to the next `end`, so a phrase can be
    /// required of ONE contract rather than of the file as a whole.
    fn slice_between<'a>(src: &'a str, start: &str, end: &str) -> &'a str {
        let from = src
            .find(start)
            .unwrap_or_else(|| panic!("region anchor not found: {start:?}"));
        let rest = &src[from..];
        let to = rest
            .find(end)
            .unwrap_or_else(|| panic!("region end anchor not found after {start:?}: {end:?}"));
        &rest[..to]
    }

    #[test]
    fn or_doc_contracts_carry_no_withdrawn_or_provenance_clause() {
        let src = normalized(production_source());

        for phrase in [
            // The WAL-window-only fold base. A snapshot frame read as the
            // checkpoint the fold starts FROM silently loses every add it
            // carries whenever the coalesced store write had not yet landed.
            "the resident slot as of the key's last full-snapshot checkpoint",
            // The window-fold shape the single-delta signature replaced.
            "checkpoint-window deltas in WAL sequence order",
            // The frontier's sole OR justification from before there were two
            // OR frame kinds. Monotonicity does not cover a delta frame.
            "for OR via snapshot monotonicity",
            // Prose provenance: which wave, gate or spec produced the code
            // belongs in the commit message, never in the contract.
            "before any real delta framing lands",
            "NOT wired to this in Wave 1",
            "resolved at the R1 `/xask` design gate",
            "This models the spec's candidate shape",
        ] {
            assert!(
                !src.contains(phrase),
                "a withdrawn or provenance clause is still in a doc-contract: {phrase:?}"
            );
        }
    }

    #[test]
    fn test_non_subsuming_doc_states_the_role_no_other_variant_fills() {
        let src = normalized(production_source());
        assert!(
            src.contains("the only `WalOp` with no replay semantics"),
            "the `TestNonSubsuming` doc must state the coverage no other variant \
             can supply, so the reason the variant survives stays checkable \
             rather than assumed by whoever next reads the enum"
        );
    }

    #[test]
    fn frontier_idempotency_contract_splits_the_two_or_frame_kinds() {
        let src = production_source();

        let regions = [
            (
                "`run`'s doc-contract",
                slice_between(
                    src,
                    "/// Replays all un-applied entries for every partition",
                    "pub async fn run(",
                ),
            ),
            (
                "the frontier loop's inline comment",
                slice_between(
                    src,
                    "// Only a success that is still contiguous with the prefix",
                    "self.first_failed.is_none()",
                ),
            ),
        ];

        for (what, region) in regions {
            let region = normalized(region);
            for needle in ["WalOp::OrDelta", "apply_or_delta"] {
                assert!(
                    region.contains(needle),
                    "{what} must name `{needle}`: a delta frame is idempotent by \
                     set-algebra fold idempotency through the live apply, NOT by \
                     the snapshot monotonicity that covers `WalOp::Store` OR frames"
                );
            }
        }
    }

    /// Every source this repository ships, keyed by the sweep id that found the
    /// no-emitter claim it used to carry.
    ///
    /// The invariant catalog is five levels above this file. Reaching it takes a
    /// package → repo-root build-time edge, and that edge is taken deliberately:
    /// the alternative is leaving the catalog's own clauses uninstrumented, which
    /// is precisely how they survived three audits while the code around them
    /// moved.
    const INVARIANTS_MD: &str = include_str!("../../../../../INVARIANTS.md");
    const WAL_FORMAT_RS: &str = include_str!("format.rs");
    const WAL_HARNESS_MOD_RS: &str = include_str!("../datastores/wal_harness/mod.rs");
    const WAL_HARNESS_CASES_RS: &str = include_str!("../datastores/wal_harness/cases.rs");
    const WRITE_BEHIND_RS: &str = include_str!("../datastores/write_behind.rs");
    const CRDT_RS: &str = include_str!("../../service/domain/crdt.rs");

    /// A no-emitter claim the sweep returned, as `(sweep id, source, needle)`.
    ///
    /// The needle is rebuilt from parts at every row without exception. That is
    /// load-bearing rather than stylistic: two rows target this very file, and a
    /// third targets a file a package-walking scan reaches, so a needle written
    /// as one literal would find ITSELF and report a claim that is not there.
    type SweptNeedle = (&'static str, fn() -> &'static str, fn() -> String);

    /// The falsified claim sites the repo-wide normalized sweep returns.
    ///
    /// Derived from the sweep, never maintained beside it. The length assertion
    /// below is what makes that derivation checkable: adding a claim site and
    /// adding a needle are otherwise independent acts, and every false green in
    /// this lineage came from exactly that independence.
    #[allow(clippy::type_complexity)]
    const SWEPT_NEEDLES: [SweptNeedle; 16] = [
        (
            "b1",
            || WAL_FORMAT_RS,
            || format!("no code path {} a delta frame", "emits"),
        ),
        ("b4", production_source, || {
            format!("no code path {} a delta frame", "emits")
        }),
        (
            "b2",
            || WAL_HARNESS_MOD_RS,
            || format!("no production path {} a delta frame yet", "emits"),
        ),
        (
            "b3",
            || WAL_HARNESS_CASES_RS,
            || format!("proof that {} emits a delta frame", "nothing"),
        ),
        (
            "b5",
            || INVARIANTS_MD,
            || format!("No {} exists yet", "emitter"),
        ),
        (
            "b6",
            || INVARIANTS_MD,
            || format!("The {} lands separately", "emitter"),
        ),
        (
            "b7",
            || INVARIANTS_MD,
            || format!("held by `WalWriter::append`'s tier-P {}", "refusal"),
        ),
        (
            "b8",
            || INVARIANTS_MD,
            || format!("fail-stopping the {} guard", "append"),
        ),
        (
            "b9",
            || WAL_HARNESS_CASES_RS,
            || format!("no production path emits a `WalOp::OrDelta` {}", "yet"),
        ),
        (
            "b10",
            || WAL_HARNESS_CASES_RS,
            || format!("This scan is not what holds the {} property", "no-emitter"),
        ),
        (
            "b11",
            || WRITE_BEHIND_RS,
            || format!("No production framing answers `false` {}", "yet"),
        ),
        ("b12", production_source, || {
            format!("a property of the CODE rather than of a {} scan", "source")
        }),
        (
            "b13",
            || CRDT_RS,
            || format!("{} in production overrides them", "NOTHING"),
        ),
        (
            "b14",
            || INVARIANTS_MD,
            || format!("no production {} site", "construction"),
        ),
        (
            "b15",
            || WAL_HARNESS_CASES_RS,
            || format!("NO production path {} a", "constructs"),
        ),
        (
            "b16",
            || WAL_HARNESS_CASES_RS,
            || format!("reader is provable before a {} exists", "writer"),
        ),
    ];

    /// A second phrase living INSIDE b1's and b4's claim text.
    ///
    /// Held apart from the swept set so it cannot inflate the parity count: an
    /// extra belt that silently raised the total would make the parity number
    /// unfalsifiable, which is the bookkeeping failure this instrument exists to
    /// close rather than to repeat.
    #[allow(clippy::type_complexity)]
    const SUPPLEMENTARY_NEEDLES: [SweptNeedle; 2] = [
        (
            "s1",
            || WAL_FORMAT_RS,
            || format!("no delta-bearing WAL exists {}", "on disk"),
        ),
        ("s2", production_source, || {
            format!("no delta-bearing WAL exists {}", "on disk")
        }),
    ];

    /// A phrase each source is KNOWN to contain, as `(source, control)`.
    ///
    /// The absence loop below is eighteen assertions that a count is zero, and a
    /// zero has two causes: the claim is gone, or the scan never read the file.
    /// A moved `include_str!` path, a truncating helper, a renamed catalog — any
    /// of them turns this instrument silently vacuous while it still reports
    /// green. Each control is a phrase whose disappearance would itself be a
    /// finding, so the control cannot rot into a tautology.
    ///
    /// Rebuilt from parts for the same reason the needles are: two of these
    /// sources are this very file.
    #[allow(clippy::type_complexity)]
    const SOURCE_CONTROLS: [(&str, fn() -> &'static str, fn() -> String); 7] = [
        (
            "INVARIANTS.md",
            || INVARIANTS_MD,
            || format!("TG-OR-{}: OR delta-fold recovery", "003"),
        ),
        ("wal/mod.rs (production prefix)", production_source, || {
            format!("pub struct Wal{} {}", "Writer", "{")
        }),
        (
            "wal/format.rs",
            || WAL_FORMAT_RS,
            || format!("pub const FRAME_{}: u8 = 1;", "VERSION"),
        ),
        (
            "wal_harness/mod.rs",
            || WAL_HARNESS_MOD_RS,
            || format!("# Crash-model limit {}", "(normative)"),
        ),
        (
            "wal_harness/cases.rs",
            || WAL_HARNESS_CASES_RS,
            || format!("fn is_delta_frame_{}(rel: &str) -> bool", "home"),
        ),
        // These two split one character EARLIER than the others on purpose. Split
        // at the brace, the row's own literal still carries the whole control as a
        // prefix, so a source mis-pointed at THIS file would satisfy its own
        // control and the mis-point would go unreported. Splitting inside the
        // identifier leaves no such prefix here — the same reason every needle
        // below is rebuilt from parts.
        (
            "datastores/write_behind.rs",
            || WRITE_BEHIND_RS,
            || format!("pub struct WriteBehindDataStor{}", "e {"),
        ),
        (
            "service/domain/crdt.rs",
            || CRDT_RS,
            || format!("pub struct CrdtServic{}", "e {"),
        ),
    ];

    /// No file in this repository asserts that no emitter exists.
    ///
    /// Every count runs on the WHITESPACE-NORMALIZED view, never on raw lines. A
    /// bare single-line search does not satisfy this: the two claims that started
    /// this lineage were both line-wrapped inside doc comments, so a raw search
    /// for them returned zero before a single character had been changed. That
    /// false green is the exact defect class this scan replaces.
    #[test]
    fn no_document_still_claims_that_nothing_emits_a_delta_frame() {
        // Positive controls FIRST, so a scan that reads nothing cannot report
        // eighteen clean absences off six empty strings.
        for (name, source, control) in SOURCE_CONTROLS
            .iter()
            .map(|(name, source, control)| (name, source(), control()))
        {
            assert!(
                normalized(source).contains(&control),
                "{name}: the scan cannot see this source -- {control:?} is missing, so every \
                 absence asserted below would be an artefact of reading the wrong bytes, not a \
                 property of the tree"
            );
        }

        assert_eq!(
            SWEPT_NEEDLES.len(),
            16,
            "the needle set is DERIVED from the repo-wide normalized sweep, not maintained \
             beside it. If the sweep now returns a different number of falsified sites, that \
             is a finding to surface -- never an array to edit until this passes again"
        );

        for (id, source, needle) in SWEPT_NEEDLES
            .iter()
            .chain(SUPPLEMENTARY_NEEDLES.iter())
            .map(|(id, source, needle)| (id, source(), needle()))
        {
            let hits = normalized(source).matches(&needle).count();
            assert_eq!(
                hits, 0,
                "{id}: a production path DOES emit delta frames now -- \
                 `write_behind::add_with_witness` is the emitter -- so the claim {needle:?} \
                 is false and must not survive anywhere in the tree"
            );
        }
    }

    /// The emitter, the retired guard and every instrument the emitter reddens
    /// move as ONE unit.
    ///
    /// Asserted in both directions, because a one-directional check is what let
    /// an emitter land beside instruments that still encoded its absence. An
    /// armed emitter beside a live append guard aborts the process on the first
    /// production OR write; a retired guard with no emitter removes a structural
    /// protection while documenting a path that does not exist; and an emitter
    /// beside an un-flipped source-scanning instrument is a CI break handed to
    /// whoever merges next. All three are the same seam.
    #[test]
    fn the_emitter_the_guard_and_every_instrument_it_reddens_move_together() {
        // The emitter's file is scanned to its inline test module and no
        // further. Its own tests name the frame freely -- they are the proof the
        // emitter works -- so scanning the whole file would let a tree whose
        // PRODUCTION emitter had been deleted still pass this check off its
        // leftover tests, which is the false green in miniature.
        let emitter_source = &WRITE_BEHIND_RS[..WRITE_BEHIND_RS
            .find("#[cfg(test)]\nmod tests {")
            .expect("the emitter's file carries its tests in an inline `mod tests`")];
        let emitter_present = normalized(emitter_source).contains(&format!(
            "fn add_with_{}",
            // Rebuilt from parts: an in-package scan counts this symbol's
            // definitions, and a literal here would be a fourth one.
            "witness"
        )) && emitter_source.contains("WalOp::OrDelta");
        assert!(
            emitter_present,
            "the write-behind store must override the witness-bearing add and frame the \
             delta -- without it every clause below documents a path that does not exist"
        );

        let guard_source = normalized(production_source());
        for residue in [
            format!("allow_or_{}", "delta"),
            format!("test_permit_one_or_delta_{}", "append"),
            format!("test_or_delta_permit_{}", "outstanding"),
        ] {
            assert!(
                !guard_source.contains(&residue),
                "the emitter is live, so {residue} must be gone: the append guard would \
                 fail-stop the process on the first production OR write"
            );
        }

        let cases = normalized(WAL_HARNESS_CASES_RS);
        assert!(
            cases.contains("src/storage/datastores/write_behind.rs"),
            "belts 2 and 3 must admit the emitter's file, or they are RED against the very \
             change that makes them meaningful"
        );

        // Both halves of the implementor scan live in ONE fn, so a whole-file
        // `contains` cannot tell them apart: `"total, 2,"` is satisfied by any
        // `2` anywhere in the file, including an unrelated one, which would let a
        // wrongly-bumped demand assertion pass the very clause written to catch
        // it. Slice the two regions and assert each against its own count.
        let crdt = normalized(CRDT_RS);
        let store_side = slice_between(
            &crdt,
            concat!("let store_", "side = ["),
            concat!("let demand = ", "concat!"),
        );
        assert!(
            store_side.contains("assert_eq!( total, 3,"),
            "the store-side implementor scan must count THREE -- the defaulted definition, \
             the one production override, and the one test spy"
        );
        assert!(
            store_side.contains("storage/datastores/write_behind.rs"),
            "the store-side scan's allowlist must admit the emitter's file, or the count of \
             three is satisfied by some other third home"
        );

        let demand = slice_between(
            &crdt,
            concat!("let demand = ", "concat!"),
            concat!("fn or_doc_contracts_carry_no_", "falsified_clause"),
        );
        assert!(
            demand.contains("assert_eq!( total, 2,"),
            "the demand-predicate scan counts a DIFFERENT symbol this spec adds no \
             implementor of, so its count must NOT have moved with the store-side one"
        );
        assert!(
            !demand.contains("assert_eq!( total, 3,"),
            "the demand-predicate scan must NOT have been bumped alongside the store-side \
             one: that would be a false edit asserting an implementor that does not exist"
        );
    }

    /// The structural half of the single-algebra rule: the fold DELEGATES, and
    /// writes no OR algebra of its own.
    ///
    /// The behavioural half — a mixed add / remove / re-add / prune window whose
    /// outcome only the live algebra produces — lives in the harness's case
    /// family. It cannot see this property: a second, hand-written copy of the
    /// algebra that currently AGREES passes every differential case while being
    /// exactly the fork `TG-OR-003` forbids, and it would diverge silently the
    /// first time the live path changed under it.
    ///
    /// ADVISORY GRADE, and deliberately so. Both halves are text assertions over
    /// one region of one file: the positive half requires the two calls in their
    /// argument-bearing form, so a prose mention of either symbol cannot satisfy
    /// it, and the negative half names the shapes a hand-rolled fold reaches for.
    /// Neither is a proof that no algebra exists — a helper called from the fold
    /// would carry its body out of this region. What they do is make the drift
    /// visible at the place a reviewer already reads.
    #[test]
    fn or_delta_fold_delegates_to_the_live_seams_and_writes_no_algebra_of_its_own() {
        let region = slice_between(
            production_source(),
            "impl OrDeltaFold for WalRecovery {",
            "fn current_millis(",
        );

        // The absence half below is satisfied by an EMPTY region, so a slice that
        // had collapsed — anchors that drifted onto adjacent text, a fold moved
        // out from between them — would read as clean rather than as broken.
        assert!(
            region.lines().count() > 10,
            "the sliced fold region is too small to be the fold: the absence assertions below \
             would pass on it vacuously"
        );

        for call in [
            "crdt::normalize_to_or_map(&mut value)",
            "crdt::apply_or_delta(delta.clone(), &mut value)",
        ] {
            assert!(
                normalized(region).contains(call),
                "the fold must reach the live OR algebra through `{call}` — the runtime write \
                 path's own two functions, in that order. A copy here is free to drift from what \
                 the write path did, and on this path drift is a silently lost mutation"
            );
        }

        // Comment lines are dropped so prose cannot turn this red; a TRAILING
        // comment on a code line survives, which is the bound this strip has.
        let code: String = region
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join(" ");
        for shape in ["records.", "tombstones.", ".retain(", ".push(", ".dedup("] {
            assert!(
                !code.contains(shape),
                "the fold body touches `{shape}` — reaching into the OR-Map's own fields is how a \
                 second add-wins / remove-wins / prune implementation starts. The fold normalizes \
                 and applies; the algebra belongs to `crdt`"
            );
        }
    }

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

        fn is_null(&self) -> bool {
            false
        }
    }

    /// A WAL in `dir` over `inner` wrapped to keep applied watermarks, as the
    /// server opens it. The WAL holds the store weakly, so the caller keeps
    /// the returned store for as long as it uses the WAL: once the store is
    /// gone, a collection tick does nothing.
    fn open_over<S: MapDataStore + 'static>(
        dir: &Path,
        policy: WalFsyncPolicy,
        inner: S,
    ) -> (Arc<WalWriter>, Arc<WatermarkKeeping<S>>) {
        let store = Arc::new(WatermarkKeeping::new(inner));
        let wal = WalWriter::open_over_store(
            dir.to_path_buf(),
            policy,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )
        .unwrap();
        (wal, store)
    }

    /// Commits the row `sequence` of `partition` to `store`, then tells `wal`
    /// that the commit returned — the order a flush pass keeps.
    async fn commit_applied(
        store: &dyn MapDataStore,
        wal: &WalWriter,
        partition: u32,
        sequence: u64,
    ) {
        store
            .commit(&FlushCommit {
                writes: Vec::new(),
                applied: vec![(partition, sequence)],
            })
            .await
            .unwrap();
        wal.mark_applied(partition, sequence).await.unwrap();
    }

    /// The applied watermark row of `partition` that `store` holds.
    fn store_row(store: &dyn MapDataStore, partition: u32) -> u64 {
        store
            .applied_watermarks()
            .unwrap()
            .expect("the store keeps applied watermarks")
            .by_partition[partition as usize]
    }

    /// Ends `wal` as a crash does: drops it and waits until no task holds it
    /// any more, so only the store and the WAL directory remain.
    async fn crash(wal: Arc<WalWriter>) {
        let weak = Arc::downgrade(&wal);
        drop(wal);
        while weak.strong_count() > 0 {
            tokio::task::yield_now().await;
        }
    }

    fn make_wal_entry(seq: u64) -> WalEntry {
        let timestamp = Timestamp {
            millis: seq,
            counter: 0,
            node_id: "n1".to_string(),
        };
        WalEntry {
            map: "m".to_string(),
            key: format!("k{seq}"),
            op: WalOp::Store {
                value: WalStorePayload::Record(RecordValue::Lww {
                    value: TgValue::String("v".to_string()),
                    timestamp: timestamp.clone(),
                }),
                expiration_time: None,
            },
            timestamp: Some(timestamp),
            sequence: seq,
        }
    }

    // -----------------------------------------------------------------------
    // TG-WAL-006 — replay merge-idempotency (LWW-scoped) direct unit tests
    // -----------------------------------------------------------------------

    /// A load-serving in-memory `MapDataStore`: unlike `ReplayStore` (records-only,
    /// `load` returns `None`), this serves back what was last written so the
    /// `replay_entry` merge gate can read the current durable value.
    #[derive(Default)]
    struct MergeProbeStore {
        data: tokio::sync::Mutex<
            std::collections::HashMap<(String, String), crate::storage::record::RecordValue>,
        >,
        /// The key of every `add`/`remove` in call order, so a recovery test can
        /// assert the REPLAY ORDER (TG-WAL-011), not just the final state.
        replay_order: tokio::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl MapDataStore for MergeProbeStore {
        async fn add(
            &self,
            map: &str,
            key: &str,
            value: &crate::storage::record::RecordValue,
            _exp: i64,
            _now: i64,
        ) -> anyhow::Result<()> {
            self.replay_order.lock().await.push(key.to_string());
            self.data
                .lock()
                .await
                .insert((map.to_string(), key.to_string()), value.clone());
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
            self.replay_order.lock().await.push(key.to_string());
            self.data
                .lock()
                .await
                .remove(&(map.to_string(), key.to_string()));
            Ok(())
        }
        async fn remove_backup(&self, _map: &str, _key: &str, _now: i64) -> anyhow::Result<()> {
            Ok(())
        }
        async fn load(
            &self,
            map: &str,
            key: &str,
        ) -> anyhow::Result<Option<crate::storage::record::RecordValue>> {
            Ok(self
                .data
                .lock()
                .await
                .get(&(map.to_string(), key.to_string()))
                .cloned())
        }
        async fn load_all(
            &self,
            _map: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, crate::storage::record::RecordValue)>> {
            Ok(Vec::new())
        }
        async fn remove_all(&self, map: &str, keys: &[String]) -> anyhow::Result<()> {
            for k in keys {
                self.remove(map, k, 0).await?;
            }
            Ok(())
        }
        async fn enumerate_leaves(
            &self,
            _map: &str,
            _is_backup: bool,
            _sink: &mut dyn crate::storage::map_data_store::LeafSink,
        ) -> anyhow::Result<()> {
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
        fn is_null(&self) -> bool {
            false
        }
    }

    fn ts(millis: u64) -> Timestamp {
        Timestamp {
            millis,
            counter: 0,
            node_id: "n".to_string(),
        }
    }

    fn lww_frame(key: &str, millis: u64) -> WalEntry {
        WalEntry {
            map: "m".to_string(),
            key: key.to_string(),
            op: WalOp::Store {
                value: WalStorePayload::Record(RecordValue::Lww {
                    value: TgValue::Int(i64::try_from(millis).unwrap()),
                    timestamp: ts(millis),
                }),
                expiration_time: None,
            },
            timestamp: None,
            sequence: 1,
        }
    }

    fn record_frame(key: &str, value: RecordValue) -> WalEntry {
        WalEntry {
            map: "m".to_string(),
            key: key.to_string(),
            op: WalOp::Store {
                value: WalStorePayload::Record(value),
                expiration_time: None,
            },
            timestamp: None,
            sequence: 1,
        }
    }

    async fn load_millis(store: &Arc<dyn MapDataStore>, key: &str) -> Option<u64> {
        match store.load("m", key).await.unwrap() {
            Some(RecordValue::Lww { timestamp, .. }) => Some(timestamp.millis),
            _ => None,
        }
    }

    /// TG-WAL-006: the `RecordValue::Lww` gate in `replay_entry` discards a frame
    /// OLDER than the current durable value, writes through ties/newer, and — with
    /// the gate off — reproduces the pre-fix blind clobber. This isolates the GATE
    /// as the fix (independent of the harness's re-replay seam).
    #[tokio::test]
    async fn replay_lww_gate_discards_older_frame_isolated() {
        let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());
        let newer = RecordValue::Lww {
            value: TgValue::Int(20),
            timestamp: ts(20),
        };

        // Gate ON: an older (ts=10) frame over a newer (ts=20) durable value is discarded.
        store.add("m", "k", &newer, 0, 0).await.unwrap();
        WalRecovery::replay_entry(&store, &lww_frame("k", 10), 0, true)
            .await
            .unwrap();
        assert_eq!(
            load_millis(&store, "k").await,
            Some(20),
            "gate ON must discard the older frame (no clobber)"
        );

        // Gate ON: a newer (ts=30) frame writes through.
        WalRecovery::replay_entry(&store, &lww_frame("k", 30), 0, true)
            .await
            .unwrap();
        assert_eq!(
            load_millis(&store, "k").await,
            Some(30),
            "gate ON must let a newer frame write through"
        );

        // Gate ON: an equal-timestamp frame writes through (ties, not strictly older).
        WalRecovery::replay_entry(&store, &lww_frame("k", 30), 0, true)
            .await
            .unwrap();
        assert_eq!(
            load_millis(&store, "k").await,
            Some(30),
            "ties write through"
        );

        // Gate OFF: the same older frame clobbers — the pre-fix blind replay.
        WalRecovery::replay_entry(&store, &lww_frame("k", 10), 0, false)
            .await
            .unwrap();
        assert_eq!(
            load_millis(&store, "k").await,
            Some(10),
            "gate OFF reproduces the blind clobber (the defect the fix closes)"
        );
    }

    /// TG-WAL-006 scope: `OrMap`/`OrTombstones` frames and cross-kind (non-Lww
    /// stored) frames BYPASS the Lww read-compare — proven directly, since the
    /// harness `ModelValue` is LWW-only and cannot drive OR frames. Also proves a
    /// `Legacy` frame keeps always-merge replay.
    #[tokio::test]
    async fn replay_or_crosskind_and_legacy_bypass_lww_gate() {
        let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());

        // (a) OR-over-OR: with the gate ON, an OR frame blind-overwrites the durable
        // OR value — the Lww read-compare never intercepts an OR-valued frame.
        let durable_or = RecordValue::OrMap {
            records: vec![],
            tombstones: vec!["B".to_string()],
        };
        store.add("m", "o", &durable_or, 0, 0).await.unwrap();
        let or_frame = RecordValue::OrMap {
            records: vec![],
            tombstones: vec!["A".to_string()],
        };
        WalRecovery::replay_entry(&store, &record_frame("o", or_frame.clone()), 0, true)
            .await
            .unwrap();
        assert_eq!(
            store.load("m", "o").await.unwrap(),
            Some(or_frame),
            "an OR frame must bypass the Lww gate and blind-overwrite (no timestamp compare)"
        );

        // (b) cross-kind: an OLDER Lww frame over a non-Lww (OR) stored value must
        // write through — the gate's inner `Some(Lww)` match fails on an OR value.
        WalRecovery::replay_entry(&store, &lww_frame("o", 1), 0, true)
            .await
            .unwrap();
        assert_eq!(
            load_millis(&store, "o").await,
            Some(1),
            "a Lww frame over a non-Lww stored value writes through (cross-kind bypass)"
        );

        // (c) legacy: a `Legacy` payload has no real timestamp (synthesized
        // zero-epoch) and must always-merge — bypass the gate even over a newer value.
        let store2: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());
        store2
            .add(
                "m",
                "k",
                &RecordValue::Lww {
                    value: TgValue::Int(50),
                    timestamp: ts(50),
                },
                0,
                0,
            )
            .await
            .unwrap();
        let legacy = WalEntry {
            map: "m".to_string(),
            key: "k".to_string(),
            op: WalOp::Store {
                value: WalStorePayload::Legacy(TgValue::Int(1)),
                expiration_time: None,
            },
            timestamp: None,
            sequence: 1,
        };
        WalRecovery::replay_entry(&store2, &legacy, 0, true)
            .await
            .unwrap();
        assert_eq!(
            store2.load("m", "k").await.unwrap(),
            Some(RecordValue::Lww {
                value: TgValue::Int(1),
                // Legacy frames with no WAL timestamp reconstruct a zero-epoch HLC
                // with an EMPTY node id (see `replay_entry`).
                timestamp: Timestamp {
                    millis: 0,
                    counter: 0,
                    node_id: String::new(),
                },
            }),
            "a legacy frame must always-merge (bypass the gate) even over a newer value"
        );
    }

    fn or_entry(tag: &str, value: i64) -> OrMapEntry {
        OrMapEntry {
            value: TgValue::Int(value),
            tag: tag.to_string(),
            timestamp: ts(u64::try_from(value).unwrap()),
        }
    }

    fn or_delta_frame(key: &str, delta: OrDelta) -> WalEntry {
        delta_frame(key, delta, 1)
    }

    /// A delta frame of map `m` at `sequence`, for the replay tests of this
    /// module and of its sibling test modules: delta frames are built only in
    /// the WAL's own files.
    pub(super) fn delta_frame(key: &str, delta: OrDelta, sequence: u64) -> WalEntry {
        WalEntry {
            map: "m".to_string(),
            key: key.to_string(),
            op: WalOp::OrDelta { delta },
            timestamp: None,
            sequence,
        }
    }

    async fn load_or(store: &Arc<dyn MapDataStore>, key: &str) -> Option<RecordValue> {
        store.load("m", key).await.unwrap()
    }

    /// The OR delta arm is a store-seeded fold: it loads the key's CURRENT durable
    /// value, folds ONE delta onto it through the live apply, and writes it back —
    /// so consecutive frames accumulate IN THE STORE and the live add-wins /
    /// remove-wins / prune algebra is what recovery reproduces.
    #[tokio::test]
    async fn replay_or_delta_folds_onto_the_durable_value() {
        let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());

        // An absent base is a legitimate first write: it folds onto an empty OR-Map.
        WalRecovery::replay_entry(
            &store,
            &or_delta_frame(
                "k",
                OrDelta::Add {
                    entry: or_entry("t1", 1),
                },
            ),
            0,
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            load_or(&store, "k").await,
            Some(RecordValue::OrMap {
                records: vec![or_entry("t1", 1)],
                tombstones: vec![],
            }),
            "an absent base is a first write, not an error"
        );

        // The NEXT frame folds onto what the previous one wrote: the store is the
        // accumulator, so remove-wins and prune see the full accumulated state.
        for delta in [
            OrDelta::Remove {
                tag: "t1".to_string(),
            },
            OrDelta::Add {
                entry: or_entry("t1", 1),
            },
        ] {
            WalRecovery::replay_entry(&store, &or_delta_frame("k", delta), 0, true)
                .await
                .unwrap();
        }
        assert_eq!(
            load_or(&store, "k").await,
            Some(RecordValue::OrMap {
                records: vec![],
                tombstones: vec!["t1".to_string()],
            }),
            "remove-wins must survive the fold: a tombstoned tag is never resurrected"
        );

        WalRecovery::replay_entry(
            &store,
            &or_delta_frame(
                "k",
                OrDelta::Prune {
                    tags: vec!["t1".to_string()],
                },
            ),
            0,
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            load_or(&store, "k").await,
            Some(RecordValue::OrMap {
                records: vec![],
                tombstones: vec![],
            }),
            "prune subtracts from the accumulated tombstone set"
        );
    }

    /// The fold normalizes its base through the LIVE normalizer before applying.
    /// Without that step `apply_or_delta` is a SILENT no-op on a legacy
    /// `OrTombstones` blob and the mutation is lost with no error at all — the one
    /// failure this assertion exists to catch.
    #[tokio::test]
    async fn replay_or_delta_normalizes_a_legacy_base() {
        let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());

        let legacy_base = RecordValue::OrTombstones {
            tags: vec!["gone".to_string()],
        };
        store.add("m", "legacy", &legacy_base, 0, 0).await.unwrap();
        WalRecovery::replay_entry(
            &store,
            &or_delta_frame(
                "legacy",
                OrDelta::Add {
                    entry: or_entry("live", 2),
                },
            ),
            0,
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            load_or(&store, "legacy").await,
            Some(RecordValue::OrMap {
                records: vec![or_entry("live", 2)],
                tombstones: vec!["gone".to_string()],
            }),
            "a legacy OrTombstones base must be normalized, not skipped: the delta lands \
             and the pre-existing tombstones survive"
        );
    }

    /// A `WalOp::Store` OR frame inside the window is an ABSOLUTE SET applied in its
    /// sequence position — a union would keep a tag live that the snapshot
    /// tombstoned — and a later delta folds onto what that frame wrote.
    #[tokio::test]
    async fn replay_or_snapshot_is_absolute_set_then_deltas_fold_onto_it() {
        let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());

        store
            .add(
                "m",
                "mixed",
                &RecordValue::OrMap {
                    records: vec![or_entry("a", 3)],
                    tombstones: vec![],
                },
                0,
                0,
            )
            .await
            .unwrap();
        let snapshot = RecordValue::OrMap {
            records: vec![],
            tombstones: vec!["a".to_string()],
        };
        WalRecovery::replay_entry(&store, &record_frame("mixed", snapshot.clone()), 0, true)
            .await
            .unwrap();
        assert_eq!(
            load_or(&store, "mixed").await,
            Some(snapshot),
            "a snapshot frame replaces the accumulated value (absolute set), never unions with it"
        );

        for delta in [
            OrDelta::Add {
                entry: or_entry("a", 3),
            },
            OrDelta::Add {
                entry: or_entry("b", 4),
            },
        ] {
            WalRecovery::replay_entry(&store, &or_delta_frame("mixed", delta), 0, true)
                .await
                .unwrap();
        }
        assert_eq!(
            load_or(&store, "mixed").await,
            Some(RecordValue::OrMap {
                records: vec![or_entry("b", 4)],
                tombstones: vec!["a".to_string()],
            }),
            "deltas after a snapshot fold onto the snapshot: the tag it tombstoned stays dead"
        );
    }

    fn remove_frame(key: &str, millis: u64) -> WalEntry {
        WalEntry {
            map: "m".to_string(),
            key: key.to_string(),
            op: WalOp::Remove,
            timestamp: Some(ts(millis)),
            sequence: 1,
        }
    }

    /// TG-WAL-009: `replay_entry` replays a `WalOp::Remove` UNCONDITIONALLY —
    /// identical to the live remove path (a blind delete). It NEVER consults the
    /// frame's timestamp and NEVER resurrects a value via a gate. The strictly-older
    /// case is the discriminator: an HLC-sourced gate (the reverted regression)
    /// would have SKIPPED a Remove whose sourced HLC is older than the durable
    /// value, losing the acked delete; unconditional replay deletes it, matching
    /// live. This isolates the SEMANTICS (independent of the harness re-replay seam),
    /// and `merge_gate` is proven irrelevant to the Remove arm.
    #[tokio::test]
    async fn replay_remove_replays_unconditionally_isolated() {
        let newer = RecordValue::Lww {
            value: TgValue::Int(20),
            timestamp: ts(20),
        };
        let sentinel_remove = WalEntry {
            map: "m".to_string(),
            key: "k".to_string(),
            op: WalOp::Remove,
            timestamp: Some(Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            }),
            sequence: 1,
        };

        // The discriminator: a Remove whose sourced HLC (10) is STRICTLY OLDER than
        // a newer durable value (20) STILL deletes. The reverted gate would have
        // skipped this (resurrecting the value and losing the acked delete); live
        // remove deletes unconditionally, so replay must too. Proven under BOTH
        // `merge_gate` values — the Remove arm ignores the flag entirely.
        for merge_gate in [true, false] {
            let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());
            store.add("m", "k", &newer, 0, 0).await.unwrap();
            WalRecovery::replay_entry(&store, &remove_frame("k", 10), 0, merge_gate)
                .await
                .unwrap();
            assert_eq!(
                store.load("m", "k").await.unwrap(),
                None,
                "a strictly-older Remove still deletes (unconditional replay = live), \
                 merge_gate={merge_gate}"
            );
        }

        // A tie, a newer, a sentinel, and a legacy `None` Remove all delete too —
        // there is no timestamp condition on the Remove arm at all.
        for frame in [
            remove_frame("k", 20),
            remove_frame("k", 30),
            sentinel_remove,
            WalEntry {
                map: "m".to_string(),
                key: "k".to_string(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: 1,
            },
        ] {
            let store: Arc<dyn MapDataStore> = Arc::new(MergeProbeStore::default());
            store.add("m", "k", &newer, 0, 0).await.unwrap();
            WalRecovery::replay_entry(&store, &frame, 0, true)
                .await
                .unwrap();
            assert_eq!(
                store.load("m", "k").await.unwrap(),
                None,
                "every Remove deletes unconditionally regardless of its timestamp: {frame:?}"
            );
        }
    }

    /// TG-WAL-011: recovery replays the unapplied window in strictly ASCENDING
    /// WAL-sequence order — `Wal::unapplied` sorts frames by sequence regardless of
    /// the order they were appended (its defensive `sort_by_key`), and `run`
    /// replays them in that order. This is premise (d) of the TG-WAL-009 gate-free
    /// warrant — the "a strictly-newer re-creation is replayed AFTER the stale
    /// Remove" step. Frames appended OUT OF ORDER (seq 3, 1, 2) come back — and
    /// replay — 1, 2, 3. Removing the `sort_by_key` in `unapplied` makes both
    /// assertions FAIL — the discriminator for the ordering contract.
    #[tokio::test]
    async fn wal_recovery_replays_in_strictly_ascending_sequence_order() {
        let dir = tempfile::tempdir().unwrap();
        let (wal, store) = open_over(dir.path(), WalFsyncPolicy::None, MergeProbeStore::default());

        // Append out of sequence order (all > 0; seq 0 is the reserved sentinel
        // `unapplied` filters). Each key carries its sequence so the probe records
        // the replay order back.
        for seq in [3u64, 1, 2] {
            let entry = WalEntry {
                map: "m".to_string(),
                key: seq.to_string(),
                op: WalOp::Remove,
                timestamp: None,
                sequence: seq,
            };
            wal.append(0, &entry).await.unwrap();
        }
        // `unapplied` sorts the shuffled appends into strictly ascending sequence
        // order — the contract `run`'s in-order replay (and TG-WAL-009) depends on.
        let enumerated: Vec<u64> = wal
            .unapplied(0)
            .await
            .unwrap()
            .iter()
            .map(|e| e.sequence)
            .collect();
        assert_eq!(
            enumerated,
            vec![1, 2, 3],
            "unapplied must return strictly ascending sequence order regardless of append order"
        );

        WalRecovery::new(Arc::clone(&wal), vec![0])
            .run(Arc::clone(&store) as Arc<dyn MapDataStore>)
            .await
            .unwrap();

        let order = store.replay_order.lock().await.clone();
        assert_eq!(
            order,
            vec!["1".to_string(), "2".to_string(), "3".to_string()],
            "recovery must replay in ascending sequence order (got {order:?})"
        );
    }

    /// Manual measurement of the per-append fsync tax of each `WalFsyncPolicy`.
    ///
    /// `#[ignore]` so it never runs in CI (it is a benchmark, not a correctness
    /// assertion) — invoke it deliberately to reproduce the durability-vs-throughput
    /// numbers behind the production-default fsync choice:
    ///
    /// ```text
    /// cargo test --release -p topgun-server --lib \
    ///   wal::tests::measure_fsync_policy_append_cost -- --ignored --nocapture
    /// ```
    ///
    /// It appends a fixed number of frames to a single partition sequentially
    /// (the worst case for `PerOp`, where every append fsyncs before returning)
    /// and reports total throughput plus p50/p99 append latency for `PerOp`,
    /// `Batched`, and `None`. Single-partition sequential isolates the raw
    /// per-fsync cost; production spreads writes across 271 partitions, so the
    /// aggregate `PerOp` penalty is amortised by cross-partition fsync parallelism.
    #[tokio::test]
    #[ignore = "benchmark; run manually with --ignored --nocapture"]
    async fn measure_fsync_policy_append_cost() {
        const N: u64 = 5000;
        const WARMUP: u64 = 200;

        async fn run(policy: WalFsyncPolicy) -> (f64, u64, u64) {
            let dir = tempfile::tempdir().unwrap();
            let (wal, _store) = open_over(dir.path(), policy, ReplayStore::default());
            // Warm up so file creation / first-write cost is excluded.
            for seq in 1..=WARMUP {
                wal.append(0, &make_wal_entry(seq)).await.unwrap();
            }
            let mut samples: Vec<u128> = Vec::with_capacity(usize::try_from(N).unwrap_or(0));
            let start = std::time::Instant::now();
            for seq in (WARMUP + 1)..=(WARMUP + N) {
                let t = std::time::Instant::now();
                wal.append(0, &make_wal_entry(seq)).await.unwrap();
                samples.push(t.elapsed().as_nanos());
            }
            let elapsed = start.elapsed();
            samples.sort_unstable();
            let p50_us = u64::try_from(samples[samples.len() / 2] / 1000).unwrap_or(u64::MAX);
            let p99_us =
                u64::try_from(samples[samples.len() * 99 / 100] / 1000).unwrap_or(u64::MAX);
            let ops_per_sec = f64::from(u32::try_from(N).unwrap()) / elapsed.as_secs_f64();
            (ops_per_sec, p50_us, p99_us)
        }

        let (perop_ops, perop_p50, perop_p99) = run(WalFsyncPolicy::PerOp).await;
        let (batched_ops, batched_p50, batched_p99) = run(WalFsyncPolicy::Batched).await;
        let (none_ops, none_p50, none_p99) = run(WalFsyncPolicy::None).await;

        println!("\n=== WAL append fsync-policy cost (single partition, {N} appends) ===");
        println!("policy    ops/sec        p50(us)   p99(us)");
        println!("PerOp     {perop_ops:>10.0}   {perop_p50:>7}   {perop_p99:>7}");
        println!("Batched   {batched_ops:>10.0}   {batched_p50:>7}   {batched_p99:>7}");
        println!("None      {none_ops:>10.0}   {none_p50:>7}   {none_p99:>7}");
        assert!(
            perop_ops > 0.0 && batched_ops > 0.0 && none_ops > 0.0,
            "degenerate run: a policy measured 0 ops/sec"
        );
        let slowdown = batched_ops / perop_ops;
        println!("Batched/PerOp throughput ratio: {slowdown:.1}x");

        // Sanity floor: fsync-every-append does strictly more work than the
        // no-fsync path on the same medium, so PerOp must not out-throughput None.
        // A PerOp faster-or-equal to None would mean fsync was silently skipped.
        // (Ignored manual bench; on a pure RAM-disk where fsync is a near-no-op
        // the two can converge — re-check this margin if running there.)
        assert!(
            perop_ops <= none_ops,
            "PerOp ({perop_ops:.0}) not slower than None ({none_ops:.0}) — fsync may be silently skipped"
        );
    }

    // -----------------------------------------------------------------------
    // AC1: append-before-ack — entry persisted (readable) before append returns;
    // fsync-durability under kill -9 depends on the policy (see WalFsyncPolicy)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn wal_writer_append_entry_is_readable_before_ack() {
        let dir = tempfile::tempdir().unwrap();
        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());

        let entry = make_wal_entry(1);
        // append must complete (durable per policy) before returning
        wal.append(0, &entry).await.unwrap();

        // unapplied must return the entry immediately after append
        let unapplied = wal.unapplied(0).await.unwrap();
        assert_eq!(unapplied.len(), 1, "Entry must be readable after append");
        assert_eq!(unapplied[0].sequence, 1);
    }

    // Partitions whose first frames arrive together must not all seal in one
    // tick, and each must still seal within 2 * SEAL_AGE plus one tick.
    #[tokio::test]
    async fn partitions_with_one_first_frame_time_seal_in_different_ticks() {
        let dir = tempfile::tempdir().unwrap();
        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
        for partition in 0..PARTITION_COUNT {
            wal.append(partition, &make_wal_entry(1)).await.unwrap();
        }

        let mut sealed_at = vec![None; PARTITION_COUNT as usize];
        let mut elapsed = std::time::Duration::ZERO;
        while elapsed < 2 * SEAL_AGE + COLLECTION_INTERVAL {
            wal.test_advance_clock(COLLECTION_INTERVAL);
            elapsed += COLLECTION_INTERVAL;
            wal.test_collect_now().await;
            for partition in 0..PARTITION_COUNT {
                let at = &mut sealed_at[partition as usize];
                if at.is_none() && !wal.test_sealed_paths(partition).await.is_empty() {
                    *at = Some(elapsed);
                }
            }
        }

        let unsealed: Vec<u32> = (0..PARTITION_COUNT)
            .filter(|p| sealed_at[*p as usize].is_none())
            .collect();
        assert!(
            unsealed.is_empty(),
            "every partition seals within 2 * SEAL_AGE plus one tick, unsealed: {unsealed:?}"
        );
        let last = PARTITION_COUNT as usize - 1;
        assert_ne!(
            sealed_at[0], sealed_at[last],
            "partitions 0 and {last} sealed in the same tick"
        );
        let mut per_tick: HashMap<std::time::Duration, usize> = HashMap::new();
        for at in sealed_at.iter().flatten() {
            *per_tick.entry(*at).or_default() += 1;
        }
        let largest = per_tick.values().copied().max().unwrap_or(0);
        assert!(
            largest <= PARTITION_COUNT as usize / 2,
            "one tick sealed {largest} of {PARTITION_COUNT} partitions: {per_tick:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Concurrency regression: seal/rotate must not drop appends that interleave
    // with it. The collection tick, with every segment aged past the seal age,
    // seals + rotates the active segment while an appender races on the same
    // partition. The watermark stays 0 so GC reclaims nothing and every appended
    // seq must survive in unapplied — proving the RULE: no acked frame is lost
    // when a rotation lands between two appends.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn compaction_does_not_drop_appends_racing_on_same_partition() {
        let dir = tempfile::tempdir().unwrap();
        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
        wal.test_advance_clock(SEAL_AGE);
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
                // Watermark 0 → the tick retains every entry but still seals and
                // rotates under the appender.
                for _ in 0..total {
                    wal.test_collect_now().await;
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
    // no acked WAL frame may be dropped by the collection tick's
    // seal/rotate/GC under concurrent same-partition append. Two variants:
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
        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
        wal.test_advance_clock(SEAL_AGE);

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
                    wal.test_collect_now().await;
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
        let (wal, store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
        wal.test_advance_clock(SEAL_AGE);

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
            let store = Arc::clone(&store);
            let frontier = Arc::clone(&acked_frontier);
            tokio::spawn(async move {
                loop {
                    let acked = frontier.load(Ordering::Acquire);
                    // Lag the watermark behind the live frontier so seal/rotate +
                    // unlink run continuously without the watermark ever reaching
                    // an unacked sequence. The row is committed to the store
                    // first, as a flush pass does, and the tick collects by it.
                    let watermark = acked.saturating_sub(LAG);
                    commit_applied(store.as_ref(), &wal, 0, watermark).await;
                    wal.test_collect_now().await;
                    tokio::task::yield_now().await;
                    if acked >= N {
                        break;
                    }
                }
            })
        };

        appender.await.unwrap();
        compactor.await.unwrap();

        // Final watermark as the store's row holds it: every acked sequence at or
        // below it is provably applied and a correct `unapplied` filter excludes it.
        let final_watermark = store_row(store.as_ref(), 0);
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
    // We assert this by running a collection tick that reclaims every sealed
    // segment while HOLDING the active-append lock the whole time. If GC took
    // the active lock, the tick would never end; the test completing proves the
    // locks are separate and that an append in flight (which would hold
    // `active`) does not serialize against GC.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn gc_does_not_block_inflight_append() {
        let dir = tempfile::tempdir().unwrap();
        let (wal, store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
        wal.test_advance_clock(SEAL_AGE);

        // Build a partition with several sealed segments below a watermark by
        // appending and sealing repeatedly. Each tick seals the active segment
        // and rotates a fresh one; watermark 0 keeps them sealed (not yet GC'd)
        // so the sealed set is populated for the GC-under-lock assertion. The
        // last tick leaves the active segment empty, so a tick does not seal it.
        for seq in 1..=8u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
            wal.test_collect_now().await;
        }
        let handle = wal.handle(0).await.unwrap();
        assert!(
            !handle.sealed.lock().await.is_empty(),
            "test setup must leave at least one sealed segment to GC"
        );
        commit_applied(store.as_ref(), &wal, 0, 8).await;

        // Hold the active-append lock for the entire GC run. An in-flight append
        // holds exactly this lock; if GC needed it, the tick would never end.
        let active_guard = handle.active.lock().await;

        tokio::time::timeout(std::time::Duration::from_secs(30), wal.test_collect_now())
            .await
            .expect("the collection tick must not wait for the active-append lock");

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

    #[test]
    fn pre_unlink_revalidation_is_a_typed_error_not_a_panic() {
        assert_eq!(
            WalWriter::test_revalidate_before_unlink(7, 100, 100),
            Ok(()),
            "a watermark that exactly covers the segment must license the unlink"
        );
        assert_eq!(
            WalWriter::test_revalidate_before_unlink(7, 101, 100),
            Err(WalGcError::WatermarkBelowSegment {
                partition: 7,
                segment_max_seq: 101,
                revalidated: 100,
            }),
            "a segment above the re-read watermark must surface as a typed error"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn torn_active_tail_truncated_on_reopen_survives_seal() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        // Phase 1: write frames 1..=3 to the active segment, then crash the
        // writer. The store survives every phase, as it survives a crash.
        let (wal, store) = open_over(&dir_path, WalFsyncPolicy::None, ReplayStore::default());
        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        crash(wal).await;
        let reopen = || {
            WalWriter::open_over_store(
                dir_path.clone(),
                WalFsyncPolicy::None,
                Arc::clone(&store) as Arc<dyn MapDataStore>,
            )
            .unwrap()
        };

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
            f.write_all(&[format::FRAME_VERSION_V2, 0xAB, 0xCD])
                .await
                .unwrap();
            f.flush().await.unwrap();
        }

        // Phase 2: reopen the writer. The open truncates the torn tail and keeps
        // the segment sealed, never appending to it again; frames 4..=6 go to a
        // new segment, which a collection tick then seals and rotates. Watermark
        // 0 keeps nothing GC'd so every frame must remain recoverable.
        let intact_len = std::fs::metadata(&active_path).unwrap().len() - 7;
        let wal = reopen();
        for seq in 4..=6u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        wal.test_advance_clock(SEAL_AGE);
        wal.test_collect_now().await;
        assert_eq!(
            wal.test_sealed_paths(0).await,
            vec![
                active_path.clone(),
                dir_path.join(segment::format_segment_filename(0, 4))
            ],
            "the torn segment is kept sealed and the new one sealed by the tick, or the test \
             shows nothing"
        );
        assert_eq!(
            std::fs::metadata(&active_path).unwrap().len(),
            intact_len,
            "the torn segment is cut to its intact prefix and never appended to"
        );
        crash(wal).await;

        // Phase 3: fresh writer over the same dir — recovery must succeed and
        // return every acked frame (1..=6). Without Fix 1, the sealed segment's
        // torn region is fatal mid-file corruption → Err here.
        let wal = reopen();
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
        let (wal, store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());

        let entry = make_wal_entry(1);
        wal.append(0, &entry).await.unwrap();

        // Verify it's in the unapplied set before marking
        let before = wal.unapplied(0).await.unwrap();
        assert_eq!(before.len(), 1);

        commit_applied(store.as_ref(), &wal, 0, 1).await;

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

        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
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

        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());
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
        let (wal, store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());

        let entry = make_wal_entry(10);
        wal.append(0, &entry).await.unwrap();

        let recovery = WalRecovery::new(Arc::clone(&wal), vec![0]);

        // First recovery run — must succeed.
        recovery
            .run(Arc::clone(&store) as Arc<dyn MapDataStore>)
            .await
            .unwrap();

        // Counts after first run.
        let first_adds = store.adds.lock().await.len();
        assert_eq!(first_adds, 1, "the first run must replay the entry");

        // Second recovery run over the same store — the first run committed the
        // entries' row with them, so they are applied; this run should replay
        // nothing.
        recovery
            .run(Arc::clone(&store) as Arc<dyn MapDataStore>)
            .await
            .unwrap();
        let second_adds = store.adds.lock().await.len() - first_adds;

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
    fn fsync_policy_parse_perop_aliases() {
        // The harness and docs use the unseparated `perop` spelling; it must
        // resolve to PerOp so the durable mode is actually applied rather than
        // silently downgraded to Batched.
        for spelling in ["perop", "PerOp", "PER_OP", "per-op", "  perop  ", "PEROP"] {
            assert_eq!(
                spelling.parse::<WalFsyncPolicy>().unwrap(),
                WalFsyncPolicy::PerOp,
                "spelling {spelling:?} must resolve to PerOp"
            );
        }
    }

    #[test]
    fn fsync_policy_parse_garbage_still_rejected() {
        // Normalization must not turn genuinely-unknown values into a silent
        // default — they must still surface as an error at startup.
        let result = "garbage".parse::<WalFsyncPolicy>();
        assert!(
            result.is_err(),
            "unknown policy 'garbage' must return Err, not a default"
        );
        assert!(
            result.unwrap_err().to_string().contains("garbage"),
            "error must name the bad value"
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
        applied: std::sync::Mutex<Vec<(u32, u64)>>,
    }

    impl InMemoryWal {
        fn new() -> Self {
            Self {
                entries: tokio::sync::Mutex::new(Vec::new()),
                applied: std::sync::Mutex::new(Vec::new()),
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
            self.applied.lock().unwrap().push((partition, sequence));
            Ok(())
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            let guard = self.entries.lock().await;
            let applied_guard = self.applied.lock().unwrap();
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

        fn durable_applied(&self, partition: u32) -> Option<u64> {
            Some(
                self.applied
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(p, _)| *p == partition)
                    .map(|(_, seq)| *seq)
                    .max()
                    .unwrap_or(0),
            )
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

    #[test]
    fn snapshot_ops_subsume_on_coalesce() {
        // Both of today's framings carry the key's COMPLETE state, so a survivor
        // makes its predecessor redundant. A future delta framing must answer
        // `false` here, and the exhaustive match is what forces the decision.
        assert!(WalOp::Remove.subsumes_on_coalesce());
        assert!(WalOp::Store {
            value: WalStorePayload::Record(RecordValue::Lww {
                value: TgValue::String("v".to_string()),
                timestamp: Timestamp {
                    millis: 1,
                    counter: 0,
                    node_id: "n1".to_string(),
                },
            }),
            expiration_time: None,
        }
        .subsumes_on_coalesce());
        assert!(WalOp::Store {
            value: WalStorePayload::Legacy(TgValue::Int(1)),
            expiration_time: None,
        }
        .subsumes_on_coalesce());
    }

    #[tokio::test]
    async fn a_partial_state_framing_neither_subsumes_nor_replays() {
        // The `false` answer is what routes a coalesce-retire to carry-forward
        // instead of early-resolve; a `_ =>` catch-all in either match would
        // silently re-bless `true` here and lose the retired frame's effect.
        assert!(
            !WalOp::TestNonSubsuming.subsumes_on_coalesce(),
            "a partial-state survivor cannot make its predecessor redundant"
        );

        // The MIXED case, evaluated on the SURVIVOR only: a retired frame that
        // WOULD subsume must not decide the route. Reading the retired op here
        // returns `true` and early-resolves the very frame carry-forward exists
        // to keep replayable.
        let retired = WalOp::Store {
            value: WalStorePayload::Record(RecordValue::Lww {
                value: TgValue::String("v".to_string()),
                timestamp: Timestamp {
                    millis: 1,
                    counter: 0,
                    node_id: "n1".to_string(),
                },
            }),
            expiration_time: None,
        };
        assert!(
            retired.subsumes_on_coalesce() && !WalOp::TestNonSubsuming.subsumes_on_coalesce(),
            "the two ops must answer differently, or the mixed case proves nothing"
        );

        let store: Arc<dyn MapDataStore> = Arc::new(ReplayStore::default());
        let mut entry = make_wal_entry(1);
        entry.op = WalOp::TestNonSubsuming;
        assert!(
            WalRecovery::replay_entry(&store, &entry, 0, true)
                .await
                .is_err(),
            "a framing with no replay semantics must surface an error, never be \
             silently absorbed by a catch-all arm"
        );
    }

    #[test]
    fn an_or_delta_survivor_does_not_subsume_on_coalesce() {
        // Read off the SURVIVOR's own framing: a per-op delta carries only its
        // own mutation, so it cannot make a predecessor redundant. `true` here
        // would early-resolve the retired frame and permanently lose the
        // mutation it carries — a delta, unlike a snapshot, never self-heals.
        assert!(
            !WalOp::OrDelta {
                delta: OrDelta::Remove {
                    tag: "tag-1".to_string(),
                },
            }
            .subsumes_on_coalesce(),
            "an OR delta survivor carries only its own mutation, so it subsumes nothing"
        );
        assert!(
            !WalOp::OrDelta {
                delta: OrDelta::Add {
                    entry: OrMapEntry {
                        value: TgValue::Int(1),
                        tag: "tag-2".to_string(),
                        timestamp: Timestamp {
                            millis: 1,
                            counter: 0,
                            node_id: "n1".to_string(),
                        },
                    },
                },
            }
            .subsumes_on_coalesce(),
            "the answer is a property of the framing, not of the mutation kind"
        );
        assert!(
            !WalOp::OrDelta {
                delta: OrDelta::Prune {
                    tags: vec!["tag-3".to_string()],
                },
            }
            .subsumes_on_coalesce(),
            "the answer is a property of the framing, not of the mutation kind"
        );
    }

    #[tokio::test]
    async fn a_flush_pass_ok_terminal_advances_the_durable_watermark() {
        use crate::storage::datastores::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};

        // `partition_for` is private to the write-behind module, so the partition
        // is located by the store row that MOVED rather than by recomputing the
        // hash — which also makes the assertion independent of the hash function.
        fn advanced_partitions(store: &dyn MapDataStore) -> Vec<u32> {
            (0..topgun_core::PARTITION_COUNT)
                .filter(|p| store_row(store, *p) >= 1)
                .collect()
        }

        let dir = tempfile::tempdir().unwrap();
        let (wal, kept) = open_over(dir.path(), WalFsyncPolicy::PerOp, ReplayStore::default());
        let inner: Arc<dyn MapDataStore> = Arc::clone(&kept) as Arc<dyn MapDataStore>;

        // Due at once, but the timer never fires within the test: the pass
        // requested below is the only pass.
        let config = WriteBehindConfig {
            write_delay_ms: 0,
            flush_interval_ms: 600_000,
            shutdown_timeout_ms: 1_000,
            ..WriteBehindConfig::default()
        };
        let store = WriteBehindDataStore::new_with_wal(
            inner,
            config,
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                // Production seeds `max_observed + 1`, so the counter never hands
                // out sequence 0 — the reserved sentinel.
                sequence_start: 1,
            }),
        );

        let value = RecordValue::Lww {
            value: TgValue::String("v".to_string()),
            timestamp: Timestamp {
                millis: 1,
                counter: 0,
                node_id: "n1".to_string(),
            },
        };
        store.add("m", "k", &value, 0, 0).await.unwrap();

        assert!(
            advanced_partitions(kept.as_ref()).is_empty(),
            "a buffered write must not have advanced anything yet"
        );

        store.soft_flush().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !store.test_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "the requested pass never closed"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }

        // Read off the store's ROW: a pass whose commit did not carry the row
        // leaves the durable watermark where it was, and only the durable read
        // sees it.
        let advanced = advanced_partitions(kept.as_ref());
        assert_eq!(
            advanced.len(),
            1,
            "an Ok terminal of a flush pass must resolve AND advance the durable watermark"
        );
        // And the copy that bounds collection must have caught up with the row:
        // an implementation that resolves the carried sequences but never calls
        // `mark_applied` moves the pending set correctly and grows the WAL
        // forever.
        let partition = advanced[0];
        assert_eq!(
            wal.durable_applied(partition),
            Some(store_row(kept.as_ref(), partition)),
            "an Ok terminal of a flush pass must call `mark_applied` with the committed row"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_skipped_gc_is_visible_on_the_metrics_scrape() {
        fn skipped_total(rendered: &str) -> f64 {
            rendered
                .lines()
                .find(|line| {
                    line.starts_with(WAL_GC_SKIPPED_COUNTER)
                        && line.contains("watermark_below_segment")
                })
                .and_then(|line| line.rsplit(' ').next())
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
        }

        let observability = crate::service::middleware::observability::init_observability();
        let dir = tempfile::tempdir().unwrap();
        let (wal, store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());

        let before = skipped_total(&observability.render_metrics());

        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        // The tick seals frames 1..=3 into one segment that the durable
        // watermark licenses; the forced tick watermark lands below its highest
        // sequence, which a store whose rows only grow never does on its own.
        commit_applied(store.as_ref(), &wal, 0, 3).await;
        wal.test_advance_clock(SEAL_AGE);
        wal.test_force_tick_watermark(0, Some(0));
        wal.test_collect_now().await;
        wal.test_force_tick_watermark(0, None);

        let rendered = observability.render_metrics();
        assert!(
            rendered.contains(&format!("# HELP {WAL_GC_SKIPPED_COUNTER}")),
            "the counter must be DESCRIBED on the scrape, or an operator sees a \
             bare number with no meaning"
        );
        // A strict increase, not equality: the recorder is process-global and a
        // parallel test may add its own skip. Nothing can decrement it.
        assert!(
            skipped_total(&rendered) > before,
            "a retained segment must be visible to an operator scraping /metrics, \
             not only to an internal read"
        );
    }

    #[tokio::test]
    async fn append_to_sealed_active_fail_stops_at_tier_p() {
        // Serialised against every other fail-stop assertion: the observation log
        // is process-global, so a concurrent tier would make the index read below
        // return someone else's entry.
        let _guard = FAIL_STOP_TEST_LOCK.lock().await;

        let dir = tempfile::tempdir().unwrap();
        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::None, ReplayStore::default());

        wal.append(0, &make_wal_entry(1)).await.unwrap();
        wal.test_seal_active_segment(0).await.unwrap();

        let before = WalWriter::test_fail_stop_observations().len();

        // Run the append on its own task so the fail-stop's test-mode panic is
        // caught by the JoinHandle instead of unwinding the test itself — the
        // tier record has to stay readable afterwards.
        let wal_clone = Arc::clone(&wal);
        let outcome = tokio::spawn(async move { wal_clone.append(0, &make_wal_entry(2)).await })
            .await
            .err();
        assert!(
            outcome.is_some_and(|e| e.is_panic()),
            "An append to a sealed active segment must fail-stop, not return"
        );

        let observed = WalWriter::test_fail_stop_observations();
        assert_eq!(
            observed.get(before),
            Some(&WalFailStopTier::P),
            "A sealed append target is a rotation-bookkeeping bug: tier P, not tier B"
        );
        assert_eq!(
            observed.len(),
            before + 1,
            "The pre-check stops BEFORE the write path, so the sequence never \
             reaches the residual (B) disposition — a second record would mean it did"
        );
    }

    /// Under `Batched` an appended frame is in the file but not fsynced until
    /// the group-commit timer runs, and that timer ends with the process. A
    /// shutdown therefore has to be able to fsync on demand.
    ///
    /// The clock is paused so the timer cannot be what empties the counts: the
    /// precondition below fails if it ever does.
    #[tokio::test(start_paused = true)]
    async fn wal_writer_sync_leaves_no_unfsynced_frame() {
        async fn unfsynced(wal: &WalWriter, partition: u32) -> u32 {
            let handle = wal.handle(partition).await.unwrap();
            let active = handle.active.lock().await;
            active.unfsynced_count()
        }

        let dir = tempfile::tempdir().unwrap();
        let (wal, _store) = open_over(dir.path(), WalFsyncPolicy::Batched, ReplayStore::default());

        wal.append(0, &make_wal_entry(1)).await.unwrap();
        wal.append(1, &make_wal_entry(2)).await.unwrap();
        wal.append(1, &make_wal_entry(3)).await.unwrap();
        assert_eq!(
            (unfsynced(&wal, 0).await, unfsynced(&wal, 1).await),
            (1, 2),
            "the appended frames must be un-fsynced before `sync`, or the test shows nothing"
        );

        wal.sync().await.unwrap();

        assert_eq!(
            (unfsynced(&wal, 0).await, unfsynced(&wal, 1).await),
            (0, 0),
            "`sync` must fsync the active segment of every partition that held a frame"
        );
    }
}

#[cfg(test)]
#[path = "store_mode_tests.rs"]
mod store_mode_tests;

#[cfg(test)]
#[path = "batched_replay_tests.rs"]
mod batched_replay_tests;

#[cfg(test)]
mod sealed_frame_tests {
    //! Sealed WAL frames over a sealed store: no (segment key, nonce) pair is
    //! used twice, a changed frame is refused rather than taken for a torn
    //! tail, and every reader of segment frames opens a sealed segment with
    //! the store's keyring (`TG-ENC-003`, `TG-ENC-005`).

    use std::collections::{BTreeSet, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::format::{decode_segment, FRAME_HEADER_LEN};
    use super::segment::{SegmentCipher, SEGMENT_HEADER_LEN};
    use super::store_mode_tests::{
        keys_in, lww, open_engine, record_key, segment_files, store_frame, CountingEngine, Node,
        Root,
    };
    use super::{Wal, WalFsyncPolicy, WalTestEvent, WalWriter, SEAL_AGE};
    use crate::storage::datastores::engine_store::{
        meta_key, EngineMapDataStore, KS_BACKUP, KS_PRIMARY, META_KEYRING,
    };
    use crate::storage::datastores::kv_engine::KvEngine;
    use crate::storage::map_data_store::MapDataStore;
    use crate::storage::record::RecordValue;
    use crate::storage::sealing::{
        self, Encryption, EncryptionMode, FileKeyProvider, Fingerprint, KeyState, KeyringRow,
        MasterKeyProvider, SEALED_MARK,
    };

    const P: u32 = 5;

    fn provider() -> Arc<dyn MasterKeyProvider> {
        Arc::new(FileKeyProvider::from_kek(
            sealing::generate_key().expect("random"),
        ))
    }

    fn encryption(
        mode: EncryptionMode,
        provider: Option<&Arc<dyn MasterKeyProvider>>,
    ) -> Encryption {
        Encryption {
            mode,
            provider: provider.cloned(),
            previous: None,
        }
    }

    /// Opens `root`'s store with `encryption`, runs its seal pass and opens
    /// the WAL over it, in the order the server starts.
    fn open_node(
        root: &Root,
        encryption: &Encryption,
        policy: WalFsyncPolicy,
    ) -> anyhow::Result<Node> {
        let engine = open_engine(root);
        let store = Arc::new(EngineMapDataStore::open_sealed(
            Arc::clone(&engine),
            encryption,
        )?);
        store.run_seal_pass()?;
        let wal = WalWriter::open_over_store(
            root.wal_dir(),
            policy,
            Arc::clone(&store) as Arc<dyn MapDataStore>,
        )?;
        Ok(Node { engine, store, wal })
    }

    fn sealed_node(root: &Root, provider: &Arc<dyn MasterKeyProvider>) -> Node {
        open_node(
            root,
            &encryption(EncryptionMode::On, Some(provider)),
            WalFsyncPolicy::PerOp,
        )
        .expect("sealed open")
    }

    /// The fingerprints of the keys of the sealed segments in `wal`'s
    /// directory; every segment there must be sealed.
    fn key_fingerprints(wal: &WalWriter) -> BTreeSet<Fingerprint> {
        let keyring = wal.keyring.as_deref().expect("a sealed node");
        let mut fingerprints = BTreeSet::new();
        for path in segment_paths(&wal.wal_dir) {
            let header = decode_segment(&std::fs::read(&path).expect("segment"))
                .header
                .expect("a v2 segment");
            assert_eq!(
                header.cipher,
                SegmentCipher::XChaCha20Poly1305,
                "{} is not sealed",
                path.display()
            );
            fingerprints.insert(
                keyring
                    .segment_key(&header)
                    .expect("its key id is in the keyring")
                    .test_fingerprint(),
            );
        }
        fingerprints
    }

    fn segment_paths(wal_dir: &Path) -> Vec<PathBuf> {
        (0..crate::storage::wal::PARTITION_COUNT)
            .flat_map(|p| segment_files(wal_dir, p).into_values())
            .collect()
    }

    /// Asserts that no `(segment key, index)` pair of a segment of `wal`'s
    /// directory was sealed twice in this process, and returns how many
    /// frames were sealed under those keys. Seals of other tests running in
    /// parallel are under other keys and left out.
    fn assert_no_pair_repeats(wal: &WalWriter) -> usize {
        let mine = key_fingerprints(wal);
        let seals = sealing::test_frame_seals()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut seen = HashSet::new();
        let mut count = 0;
        for (key, index) in seals.into_iter().filter(|(key, _)| mine.contains(key)) {
            assert!(
                seen.insert((key, index)),
                "frame index {index} was sealed twice under one segment key"
            );
            count += 1;
        }
        count
    }

    async fn append(node: &Node, partition: u32, seq: u64) {
        node.append(partition, &format!("k{seq}"), &lww("v", seq), seq)
            .await;
    }

    async fn unapplied(node: &Node, partition: u32) -> anyhow::Result<Vec<u64>> {
        Ok(node
            .wal
            .unapplied(partition)
            .await?
            .iter()
            .map(|e| e.sequence)
            .collect())
    }

    fn last_segment(wal_dir: &Path, partition: u32) -> PathBuf {
        segment_files(wal_dir, partition)
            .into_values()
            .next_back()
            .expect("a segment")
    }

    /// The offset of every frame of the segment file `data`.
    fn frame_offsets(data: &[u8]) -> Vec<usize> {
        let mut offsets = Vec::new();
        let mut at = SEGMENT_HEADER_LEN;
        while at + FRAME_HEADER_LEN <= data.len() {
            offsets.push(at);
            let len = u32::from_be_bytes(data[at + 5..at + 9].try_into().unwrap()) as usize;
            at += FRAME_HEADER_LEN + len;
        }
        offsets
    }

    /// AC8 (`TG-ENC-005`): many tasks append to one sealed partition at
    /// once. Each frame is sealed under the append lock with the index taken
    /// there, so no (key, index) pair repeats and recovery opens every frame
    /// at its position.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn ac8_concurrent_sealed_appends_seal_each_index_once() {
        const TASKS: u64 = 8;
        const PER_TASK: u64 = 300;
        let root = Root::new();
        let provider = provider();
        let node = open_node(
            &root,
            &encryption(EncryptionMode::On, Some(&provider)),
            WalFsyncPolicy::None,
        )
        .expect("sealed open");
        let next = Arc::new(AtomicU64::new(1));
        let mut tasks = Vec::new();
        for _ in 0..TASKS {
            let wal = Arc::clone(&node.wal);
            let next = Arc::clone(&next);
            tasks.push(tokio::spawn(async move {
                for _ in 0..PER_TASK {
                    let seq = next.fetch_add(1, Ordering::SeqCst);
                    wal.append(P, &store_frame(&format!("k{seq}"), &lww("v", seq), seq))
                        .await
                        .expect("append");
                }
            }));
        }
        for task in tasks {
            task.await.expect("task");
        }
        assert_eq!(
            assert_no_pair_repeats(&node.wal) as u64,
            TASKS * PER_TASK,
            "every frame was sealed once"
        );
        assert_eq!(
            unapplied(&node, P)
                .await
                .expect("recovery opens every frame"),
            (1..=TASKS * PER_TASK).collect::<Vec<_>>()
        );
        node.close().await;
    }

    /// AC8 (`TG-ENC-005`): the first opens of a sealed partition by an
    /// append and by the collection tick race past the installed-map miss.
    /// The open mutex lets one of them create the new segment.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac8_racing_first_opens_of_a_sealed_partition_create_one_segment() {
        let root = Root::new();
        let provider = provider();
        let node = sealed_node(&root, &provider);
        append(&node, P, 1).await;
        node.close().await;

        let node = sealed_node(&root, &provider);
        let created = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&created);
        node.wal.test_set_observer(Some(Arc::new(move |event| {
            if matches!(event, WalTestEvent::Created { partition: P, .. }) {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        })));
        node.wal
            .test_set_open_barrier(Some(Arc::new(tokio::sync::Barrier::new(2))));
        let wal = Arc::clone(&node.wal);
        let append_task =
            tokio::spawn(async move { wal.append(P, &store_frame("k2", &lww("v", 2), 2)).await });
        let wal = Arc::clone(&node.wal);
        let tick = tokio::spawn(async move { wal.test_collect_now().await });
        append_task.await.expect("append task").expect("append");
        tick.await.expect("tick task");
        node.wal.test_set_open_barrier(None);
        assert_eq!(
            created.load(Ordering::SeqCst),
            1,
            "the racing opens created more than one segment"
        );
        assert_eq!(unapplied(&node, P).await.expect("recovery"), vec![1, 2]);
        assert_no_pair_repeats(&node.wal);
        node.close().await;
        let node = sealed_node(&root, &provider);
        assert_eq!(unapplied(&node, P).await.expect("recovery"), vec![1, 2]);
        node.close().await;
    }

    /// AC8 (`TG-ENC-005`): a sealed segment cut at a frame boundary below its
    /// last fsync, or inside a frame, is never appended to after the restart,
    /// so the indices of the frames the cut removed are never sealed again
    /// under its key; recovery reads every frame the cut left intact and
    /// every frame appended after.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac8_a_cut_sealed_segment_is_never_appended_to() {
        for mid_frame in [false, true] {
            let root = Root::new();
            let provider = provider();
            let node = sealed_node(&root, &provider);
            append(&node, P, 1).await;
            append(&node, P, 2).await;
            let path = last_segment(&root.wal_dir(), P);
            let boundary = std::fs::metadata(&path).unwrap().len();
            append(&node, P, 3).await;
            node.close().await;

            let cut = if mid_frame { boundary + 5 } else { boundary };
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(cut)
                .unwrap();

            let node = sealed_node(&root, &provider);
            append(&node, P, 4).await;
            append(&node, P, 5).await;
            assert_eq!(
                std::fs::metadata(&path).unwrap().len(),
                boundary,
                "the cut segment was written after the restart (mid_frame={mid_frame})"
            );
            assert_eq!(segment_files(&root.wal_dir(), P).len(), 2);
            assert_no_pair_repeats(&node.wal);
            node.close().await;

            let node = sealed_node(&root, &provider);
            assert_eq!(
                unapplied(&node, P).await.expect("recovery"),
                vec![1, 2, 4, 5],
                "mid_frame={mid_frame}"
            );
            node.close().await;
        }
    }

    /// AC8 (`TG-ENC-005`): a rotation whose directory fsync fails leaves its
    /// new sealed segment behind with no frame; the next rotation replaces or
    /// passes it, and no (key, index) pair repeats.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac8_a_sealed_rotation_with_a_failed_directory_fsync_then_the_next() {
        let root = Root::new();
        let provider = provider();
        let node = sealed_node(&root, &provider);
        let rotate = |fail: bool| {
            let wal = Arc::clone(&node.wal);
            async move {
                if fail {
                    wal.test_fail_next_rotation_dir_fsync();
                }
                wal.test_advance_clock(2 * SEAL_AGE + std::time::Duration::from_secs(1));
                wal.test_collect_now().await;
            }
        };
        append(&node, P, 1).await;
        rotate(true).await;
        assert_eq!(segment_files(&root.wal_dir(), P).len(), 2);
        rotate(false).await;
        append(&node, P, 2).await;
        rotate(true).await;
        append(&node, P, 3).await;
        rotate(false).await;
        append(&node, P, 4).await;
        assert_no_pair_repeats(&node.wal);
        node.close().await;

        let node = sealed_node(&root, &provider);
        assert_eq!(
            unapplied(&node, P).await.expect("recovery"),
            vec![1, 2, 3, 4]
        );
        assert_no_pair_repeats(&node.wal);
        node.close().await;
    }

    /// Flips a byte of the payload of frame `frame` of the segment at `path`
    /// and writes the CRC of the changed payload, so only authentication can
    /// tell. Returns the frame's offset.
    fn tamper(path: &Path, frame: usize) -> usize {
        let mut data = std::fs::read(path).expect("segment");
        let at = frame_offsets(&data)[frame];
        let len = u32::from_be_bytes(data[at + 5..at + 9].try_into().unwrap()) as usize;
        let payload = at + FRAME_HEADER_LEN..at + FRAME_HEADER_LEN + len;
        data[payload.start + 1] ^= 0x01;
        let crc = crc32c::crc32c(&data[payload]);
        data[at + 9..at + FRAME_HEADER_LEN].copy_from_slice(&crc.to_be_bytes());
        std::fs::write(path, data).expect("write segment");
        at
    }

    /// AC10 (`TG-ENC-003`): a changed payload byte with its CRC fixed is
    /// refused by recovery naming the file and the offset — in the last frame
    /// of the last segment, where a torn tail would be tolerated, and in a
    /// middle frame; a cut inside the last frame is tolerated as a torn tail.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac10_a_changed_sealed_frame_is_refused_and_a_torn_one_tolerated() {
        for case in ["last frame", "middle frame", "torn last frame"] {
            let root = Root::new();
            let provider = provider();
            let node = sealed_node(&root, &provider);
            for seq in 1..=3 {
                append(&node, P, seq).await;
            }
            node.close().await;
            let path = last_segment(&root.wal_dir(), P);
            let offset = match case {
                "last frame" => tamper(&path, 2),
                "middle frame" => tamper(&path, 1),
                _ => {
                    let len = std::fs::metadata(&path).unwrap().len();
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&path)
                        .unwrap()
                        .set_len(len - 3)
                        .unwrap();
                    0
                }
            };

            let node = sealed_node(&root, &provider);
            if case == "torn last frame" {
                assert_eq!(unapplied(&node, P).await.expect("tolerated"), vec![1, 2]);
                node.recover()
                    .await
                    .expect("recovery replays the intact prefix");
            } else {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                for error in [
                    unapplied(&node, P).await.expect_err(case),
                    node.recover().await.expect_err(case),
                ] {
                    let message = format!("{error:#}");
                    assert!(
                        message.contains(&name)
                            && message.contains(&format!("offset {offset}"))
                            && message.contains("fails authentication"),
                        "{case}: {message}"
                    );
                }
            }
            node.close().await;
        }
    }

    /// `AC12b` (`TG-ENC-005`): a sealed node holding a sealed segment with a
    /// frame above the watermark in a partition not written in this run
    /// stops through `finish(true)` before any collection tick; `finish`
    /// reads that segment with the keyring, keeps it, and the frame replays
    /// at the next start.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac12b_finish_keeps_a_sealed_segment_above_the_watermark() {
        let root = Root::new();
        let provider = provider();
        let node = sealed_node(&root, &provider);
        append(&node, P, 1).await;
        node.close().await;
        let path = last_segment(&root.wal_dir(), P);

        let node = sealed_node(&root, &provider);
        assert!(
            !node.wal.partitions.lock().await.contains_key(&P),
            "the partition has an installed handle before finish"
        );
        node.wal.finish(true).await.expect("finish");
        assert!(
            path.exists(),
            "finish unlinked a sealed segment above the watermark"
        );
        node.close().await;

        let node = sealed_node(&root, &provider);
        node.recover().await.expect("recovery");
        assert_eq!(node.engine.row(P), Some(1), "the frame replayed");
        assert!(node
            .engine
            .get(&record_key(&node.engine, "k1"))
            .expect("engine read")
            .is_some());
        node.close().await;
    }

    /// Every record value of `engine`, primary and backup rows.
    fn values(engine: &CountingEngine) -> Vec<Vec<u8>> {
        engine
            .dump()
            .into_iter()
            .filter(|(key, _)| matches!(key.first(), Some(&KS_PRIMARY | &KS_BACKUP)))
            .map(|(_, value)| value)
            .collect()
    }

    fn keyring_row(engine: &impl KvEngine) -> KeyringRow {
        KeyringRow::decode(
            &engine
                .get(&meta_key(META_KEYRING))
                .expect("engine read")
                .expect("the keyring row is kept"),
        )
        .expect("a keyring row")
    }

    /// AC12 (`TG-ENC-001`, `TG-ENC-005`): with sealed values in the store and
    /// sealed frames left unapplied by a crash, a restart with encryption off
    /// replays the frames, ends its unseal pass with every value plain and
    /// keeps the keyring with every key decrypt-only; a restart off with no
    /// master key refuses; a restart on adds key 2, never reusing 1, and
    /// seals every value under it.
    #[tokio::test(flavor = "multi_thread")]
    async fn ac12_disable_with_sealed_frames_on_disk() {
        let root = Root::new();
        let provider = provider();
        let keys = keys_in(P, 3);
        let node = sealed_node(&root, &provider);
        let committed: Vec<(String, RecordValue)> = ["a", "b", "c"]
            .iter()
            .map(|k| ((*k).to_string(), lww("sealed", 1)))
            .collect();
        node.commit(&committed, &[]).await.expect("commit");
        for (seq, key) in (1..).zip(&keys) {
            node.append(P, key, &lww("framed", seq), seq).await;
        }
        assert!(values(&node.engine)
            .iter()
            .all(|v| sealing::sealed_key_id(v) == Some(1)));
        // A crash: no shutdown, no finish.
        node.close().await;

        let off = encryption(EncryptionMode::Off, Some(&provider));
        let node = open_node(&root, &off, WalFsyncPolicy::PerOp).expect("open with off");
        node.recover().await.expect("the sealed frames replay");
        for key in &keys {
            assert!(
                node.engine
                    .get(&record_key(&node.engine, key))
                    .expect("engine read")
                    .is_some(),
                "{key} replayed"
            );
        }
        let plain = values(&node.engine);
        assert_eq!(plain.len(), 6);
        assert!(plain.iter().all(|v| v.first() != Some(&SEALED_MARK)));
        let row = keyring_row(&*node.engine);
        assert_eq!(row.keys.len(), 1);
        assert!(row.keys.iter().all(|k| k.state == KeyState::DecryptOnly));
        // New segments are plain once no key is active.
        append(&node, P, 4).await;
        let header = decode_segment(&std::fs::read(last_segment(&root.wal_dir(), P)).unwrap())
            .header
            .expect("a v2 segment");
        assert_eq!(header.cipher, SegmentCipher::None);
        node.close().await;

        let refused = open_node(
            &root,
            &encryption(EncryptionMode::Off, None),
            WalFsyncPolicy::PerOp,
        )
        .err()
        .expect("off with no master key refuses");
        assert!(
            format!("{refused:#}").contains("encryption: refusing to start:"),
            "{refused:#}"
        );

        let node = sealed_node(&root, &provider);
        node.recover().await.expect("recovery");
        let row = keyring_row(&*node.engine);
        let states: Vec<(u32, KeyState)> = row.keys.iter().map(|k| (k.id, k.state)).collect();
        assert_eq!(
            states,
            vec![(1, KeyState::DecryptOnly), (2, KeyState::Active)]
        );
        assert_eq!(row.next_key_id, 3);
        let sealed = values(&node.engine);
        assert_eq!(sealed.len(), 7);
        assert!(sealed.iter().all(|v| sealing::sealed_key_id(v) == Some(2)));
        node.close().await;
    }
}
