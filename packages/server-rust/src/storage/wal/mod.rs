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
use crate::storage::record::{OrMapEntry, RecordValue};

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
    ///   [`WalRecovery::replay_entry`] arm returns an error rather than applying
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
    /// Forces the pre-unlink watermark re-read to return this value instead of
    /// the sidecar's.
    ///
    /// The re-validation it drives cannot fire in production — the sidecar is
    /// monotonic, so a re-read can only return a value at or above the local
    /// watermark — so this seam is the only way to exercise that path at all.
    /// Per-writer rather than process-global so concurrent tests cannot perturb
    /// each other.
    #[cfg(test)]
    gc_revalidation_override: std::sync::Mutex<Option<u64>>,
    /// TG-WAL-003 crash-point knob (R7): when `PreUnlink`, `mark_applied`
    /// returns right after the sidecar watermark is durable but before any
    /// sealed segment is unlinked, so a harness case can end the incarnation
    /// exactly there.
    #[cfg(test)]
    gc_crash_point: std::sync::Mutex<GcCrashPoint>,
    /// TG-WAL-003 negative-control knob (R6): when `UnlinkThenFsync`,
    /// `mark_applied` unlinks sealed segments BEFORE the sidecar watermark is
    /// durable — the inverted, pre-fix order this module's comments exist to
    /// prevent — so the harness can prove its oracle actually detects the
    /// hazard rather than passing vacuously.
    #[cfg(test)]
    gc_order_mode: std::sync::Mutex<GcOrderMode>,
}

/// TG-WAL-003 crash-point seam (R7): where `mark_applied` may end an
/// incarnation while re-introducing the GC crash point.
///
/// `#[cfg(test)]`-only by construction — this type does not exist in a
/// release build, so it cannot add a runtime branch there.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GcCrashPoint {
    /// Normal behaviour: `mark_applied` runs to completion.
    #[default]
    None,
    /// Return immediately after the sidecar watermark write+fsync, before the
    /// unlink loop runs: the sidecar is durable, the sealed segments it
    /// covers are still on disk.
    PreUnlink,
}

/// TG-WAL-003 negative-control seam (R6): the order `mark_applied` performs
/// the sidecar durability step and the sealed-segment unlink step in.
///
/// `#[cfg(test)]`-only by construction — this type does not exist in a
/// release build, so it cannot add a runtime branch there.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GcOrderMode {
    /// Production order: the sidecar watermark is durable (written + fsynced)
    /// BEFORE any sealed segment is unlinked.
    #[default]
    FsyncThenUnlink,
    /// Inverted, pre-fix order re-introduced as a negative control: sealed
    /// segments are unlinked BEFORE the sidecar watermark is made durable.
    /// A crash between the two steps loses the only record that those
    /// segments' frames were already applied.
    UnlinkThenFsync,
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
            #[cfg(test)]
            gc_revalidation_override: std::sync::Mutex::new(None),
            #[cfg(test)]
            gc_crash_point: std::sync::Mutex::new(GcCrashPoint::None),
            #[cfg(test)]
            gc_order_mode: std::sync::Mutex::new(GcOrderMode::FsyncThenUnlink),
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
    /// Re-reads the partition's applied watermark for the pre-unlink check.
    ///
    /// The `.applied` sidecar is this module's only watermark source: write-behind
    /// holds the WAL, never the reverse, so its in-memory tracker is not reachable
    /// from here and reading it would invert the dependency.
    fn revalidated_watermark(&self, partition: u32) -> u64 {
        #[cfg(test)]
        if let Some(forced) = *self
            .gc_revalidation_override
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return forced;
        }
        Self::read_applied_sequence(&self.applied_path(partition))
    }

    /// The partition's crash-durable applied watermark as recorded in the
    /// `.applied` sidecar — the value that actually determines which frames
    /// [`Wal::unapplied`] still returns. Distinct from write-behind's recomputed
    /// prefix-complete watermark: this is the PERSISTED value a defective advance
    /// writes (the C13 inclusive-off-by-one over-advances it), so a test oracle
    /// can tell an over-advanced watermark that FILTERED a frame from a segment
    /// unlinked before its sidecar was durable.
    #[cfg(test)]
    pub(crate) fn test_applied_watermark(&self, partition: u32) -> u64 {
        Self::read_applied_sequence(&self.applied_path(partition))
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
        let frame = format::encode(entry)?;

        let handle = self.handle(partition).await?;
        // Take ONLY the active-append lock. GC of sealed segments runs under a
        // distinct lock and never blocks this path; rotation briefly takes this
        // same lock, which is correct — an append and a rotate on the same
        // partition must serialize so a seal never freezes a half-written frame.
        let mut active = handle.active.lock().await;

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
            }
            WalFsyncPolicy::Batched => {
                // Flush immediately when the batch threshold is reached so
                // high-throughput bursts don't wait for the timer task.
                if count >= 100 {
                    active.sync_data().await.unwrap_or_else(|e| {
                        wal_fail_stop(
                            WalFailStopTier::B,
                            &format!(
                                "WAL batched fsync failed: partition={partition}, sequence={}: {e}",
                                entry.sequence
                            ),
                        )
                    });
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

    /// Advances the partition's applied watermark and reclaims what it covers.
    ///
    /// # Control flow — three independent steps, each with its OWN precondition
    ///
    /// The local watermark is `max(sequence, current)`, so an under-advancing
    /// `sequence` collapses it to the value already on disk. That is what makes
    /// an under-advance harmless — NOT the `if watermark > current` guard, which
    /// governs only the first step:
    ///
    /// 1. **Sidecar write + its fsync** — gated by `if watermark > current`. A
    ///    watermark that does not move writes nothing and syncs nothing.
    /// 2. **Seal + rotate** — NOT gated by that guard. Its own precondition is
    ///    `active.has_frames()`: an empty active segment means no seal, no
    ///    rotate, and ZERO fsyncs; a non-empty one costs an `sync_data` plus a
    ///    parent-dir fsync.
    /// 3. **Segment GC** — NOT gated by that guard either. It reclaims sealed
    ///    segments whose highest sequence is at or below the local `watermark`,
    ///    so under an under-advance it unlinks only what the persisted sidecar
    ///    already licenses. Its parent-dir fsync has its own precondition,
    ///    `!reclaimed.is_empty()`, so a reclaiming GC adds a THIRD fsync.
    ///
    /// Reading the guard as gating steps 2 and 3 is wrong in both directions: it
    /// overstates the guard and understates the fsync cost.
    ///
    /// # Durability rule
    ///
    /// `sequence` must be prefix-complete — see the trait's contract. GC unlinks
    /// frames on the strength of this watermark, so a watermark that runs ahead
    /// of what the inner store has durably applied discards the only copy of
    /// those writes.
    ///
    /// # Errors
    ///
    /// Returns an error if the sidecar cannot be written, a segment cannot be
    /// opened, sealed, or unlinked, or a directory fsync fails. A segment that
    /// fails the pre-unlink re-validation is NOT an error: it is retained,
    /// counted, and logged.
    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
        let applied_path = self.applied_path(partition);
        // Advance the watermark monotonically; never regress it.
        let current = Self::read_applied_sequence(&applied_path);
        let watermark = sequence.max(current);

        // TG-WAL-003 negative control (R6): when forced, run the inverted,
        // pre-fix order — unlink sealed segments BEFORE the sidecar watermark
        // is durable — so the harness can prove its oracle actually detects
        // the hazard the production order below exists to prevent.
        // `#[cfg(test)]`-only: this whole block does not exist in a release
        // build, so the production path is unconditional there.
        #[cfg(test)]
        if *self
            .gc_order_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == GcOrderMode::UnlinkThenFsync
        {
            let handle = self.handle(partition).await?;
            self.seal_and_rotate_if_needed(partition, &handle).await?;
            // No durable sidecar exists yet in this mode, so there is nothing
            // to re-read: reclaim against the in-memory `watermark` directly,
            // mirroring the pre-fix code that had no durable value to
            // re-validate against.
            let reclaimed = self
                .select_reclaimable(partition, &handle, watermark, watermark)
                .await;
            self.unlink_reclaimed(&reclaimed).await?;

            // Same `gc_crash_point` knob as the production path (R7), but its
            // landing spot mirrors the inverted step order: here it fires
            // AFTER the wrongly-early unlink and BEFORE the (deferred)
            // sidecar write, instead of before the unlink. That is the exact
            // TG-WAL-003 loss window — segments are already gone from disk
            // while the sidecar is still at its OLD value, which under-seeds
            // max_observed_sequence on restart and drops/reuses a sequence
            // (see the comment on the production write below). Ending the
            // incarnation here, rather than after a fully-completed call, is
            // what makes AC7(b) reachable: a crash the harness fires only
            // between ops would otherwise never observe an inconsistent
            // on-disk state, since a completed call leaves both steps
            // durable regardless of the order they ran in.
            if *self
                .gc_crash_point
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                == GcCrashPoint::PreUnlink
            {
                return Ok(());
            }

            if watermark > current {
                Self::write_applied_sequence(&applied_path, watermark).map_err(|e| {
                    anyhow::anyhow!(
                        "Cannot write applied sidecar {}: {e}",
                        applied_path.display()
                    )
                })?;
            }
            return Ok(());
        }

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

        self.seal_and_rotate_if_needed(partition, &handle).await?;

        // TG-WAL-003 crash-point seam (R7): fires exactly between the durable
        // sidecar advance above and the unlink loop below. A harness case
        // configured with `GcCrashPoint::PreUnlink` ends the incarnation
        // here — the watermark is durable, no sealed segment has been
        // unlinked yet. `#[cfg(test)]`-only: no branch in a release build.
        #[cfg(test)]
        if *self
            .gc_crash_point
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == GcCrashPoint::PreUnlink
        {
            return Ok(());
        }

        // --- GC sealed segments fully covered by the watermark ---
        // Every candidate is re-validated against a FRESH read of the `.applied`
        // sidecar immediately before its unlink. That re-read cannot fall below
        // the local `watermark` while the sidecar stays monotonic, so this is
        // defense-in-depth, not a live hole: it converts any future watermark
        // regression from a silent unlink into a counted, logged retention.
        let revalidated = self.revalidated_watermark(partition);
        let reclaimed = self
            .select_reclaimable(partition, &handle, watermark, revalidated)
            .await;
        self.unlink_reclaimed(&reclaimed).await?;

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
    /// Seals the active segment and rotates a fresh one in, if the active
    /// segment actually holds frames.
    ///
    /// Sealing an empty active segment would leave a zero-length sealed file
    /// the next append could never reach and GC would have to special-case,
    /// so this is a no-op when the active segment is empty. Shared by both
    /// the production `mark_applied` order and the `#[cfg(test)]`
    /// order-inversion seam, so both exercise identical seal/rotate
    /// behaviour and only the watermark-vs-unlink ordering differs.
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
        Ok(())
    }

    /// Selects the prefix of sealed segments safe to reclaim given the
    /// locally computed `watermark` and a `revalidated` value to gate the
    /// unlink against.
    ///
    /// On the production path `revalidated` is a fresh disk re-read of the
    /// `.applied` sidecar (defense-in-depth: it cannot fall below `watermark`
    /// once the sidecar write above has completed). The `#[cfg(test)]`
    /// order-inversion seam instead passes the in-memory `watermark` itself,
    /// since no durable sidecar exists yet at that point in the inverted
    /// order — mirroring the pre-fix code that had no durable value to
    /// re-validate against.
    ///
    /// Takes ONLY the sealed-set lock; never the active-append lock. Appends
    /// in flight to the active segment are not blocked by this deletion.
    async fn select_reclaimable(
        &self,
        partition: u32,
        handle: &Arc<PartitionHandle>,
        watermark: u64,
        revalidated: u64,
    ) -> Vec<PathBuf> {
        let mut sealed = handle.sealed.lock().await;
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
        sealed
            .drain(..reclaim)
            .map(|s| s.path().to_path_buf())
            .collect()
    }

    /// Unlinks the segments `select_reclaimable` selected, then fsyncs the
    /// WAL directory once so the reclamation itself is durable.
    ///
    /// Per-unlink dir fsync is unnecessary: GC is resumable and replay is
    /// idempotent under the watermark filter, so a crash between unlinks is
    /// safe.
    async fn unlink_reclaimed(&self, reclaimed: &[PathBuf]) -> anyhow::Result<()> {
        for path in reclaimed {
            tokio::fs::remove_file(path).await.map_err(|e| {
                anyhow::anyhow!("Cannot unlink sealed WAL segment {}: {e}", path.display())
            })?;
        }
        if !reclaimed.is_empty() {
            fsync_dir(&self.wal_dir)?;
        }
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
    /// Whether the `RecordValue::Lww` merge-idempotency gate in `replay_entry` is
    /// active. Always `true` in production; the harness flips it off to reproduce
    /// the pre-`TG-WAL-006` blind clobber.
    replay_merge_gate: bool,
    /// Whether `run` re-replays each partition's oldest un-applied frame once more
    /// after the in-order pass — a harness seam that manufactures the "older frame
    /// re-applied over a newer durable value" condition the live `flush_key` path
    /// produces but generated ops cannot. `#[cfg(test)]`-only: it does not exist in
    /// a production build.
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
        }
    }

    /// Test-only: disable (or re-enable) the `RecordValue::Lww` merge gate in
    /// `replay_entry`. Disabling it reproduces the pre-`TG-WAL-006` blind replay.
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

    /// Replays ONE entry through the inner store.
    ///
    /// Goes straight to the store rather than through the CRDT service layer, so
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
    /// key's CURRENT durable value, folds THIS ONE delta onto it via
    /// [`OrDeltaFold::fold`], and writes the result back, so the durable store —
    /// never a WAL-window checkpoint — is both the fold base and the accumulator,
    /// and each frame is applied in its own sequence position inside the single
    /// replay stream rather than batched per key. Re-replay is a semantic no-op
    /// because the folded algebra IS the live one and its ops are idempotent set
    /// ops (retain, dedup push, subtract) — not because a delta is a snapshot. A
    /// delta carries only its own mutation, which is why it can never be skipped
    /// or dropped: a dropped remove resurrects an entry, and unlike a full-snapshot
    /// frame a delta is never repaired by a later frame.
    ///
    /// A `WalOp::Store` frame carrying an OR value stays a SUPPORTED input and is
    /// applied in its sequence position as an ABSOLUTE SET, never as a fold-base
    /// skip hint and never as a union — see the arm for why the snapshot's
    /// completeness is what licenses the replace.
    async fn replay_entry(
        inner_store: &Arc<dyn MapDataStore>,
        entry: &WalEntry,
        now: i64,
        merge_gate: bool,
    ) -> anyhow::Result<()> {
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
                // to compare; if the current durable value is a strictly-newer Lww
                // value, this frame is stale (it was superseded by a write outside
                // the replay window) and must be discarded rather than clobber it.
                // Legacy frames, OR frames, and a non-Lww stored value all fall
                // through to the blind replay unchanged (see the doc-contract above).
                // `merge_gate` is always `true` in production; the harness flips it
                // off to reproduce the pre-fix blind clobber.
                if merge_gate {
                    if let WalStorePayload::Record(RecordValue::Lww {
                        timestamp: incoming_ts,
                        ..
                    }) = value
                    {
                        if let Some(RecordValue::Lww {
                            timestamp: stored_ts,
                            ..
                        }) = inner_store.load(&entry.map, &entry.key).await?
                        {
                            if *incoming_ts < stored_ts {
                                return Ok(());
                            }
                        }
                    }
                }

                // ABSOLUTE SET, in this frame's sequence position — for an OR
                // value as much as for an LWW one. `add` REPLACES the key's
                // value with this frame's; it never unions the frame with the
                // value the store already holds, and it is never a "skip the
                // fold up to here" hint. A delta frame later in the window then
                // folds onto what this frame just wrote.
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
                inner_store.remove(&entry.map, &entry.key, now).await
            }
            WalOp::OrDelta { delta } => {
                // Store-seeded, one frame at a time: load the key's CURRENT
                // durable value, fold this ONE delta onto it, write it back. The
                // durable store is the accumulator, so nothing here collects a
                // key's deltas out of the replay stream — the next frame for this
                // key reads what this one just wrote, exactly as the live path's
                // read-modify-write does.
                let base = inner_store.load(&entry.map, &entry.key).await?;
                let folded = <Self as OrDeltaFold>::fold(base, delta);

                // Expiration `0` ("no expiry"): `load` returns no TTL and `add`
                // requires one, and a delta frame carries no expiration field —
                // it records a mutation, not a whole slot. `0` matches the live
                // OR write path, which writes both OR closures with
                // `ExpiryPolicy::NONE`, so the fold introduces no TTL drift. An
                // OR path that starts writing TTLs would need the frame to carry
                // one; it is not a value this arm may invent.
                inner_store
                    .add(&entry.map, &entry.key, &folded, 0, now)
                    .await
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
    /// `replay_entry` reads the current durable value and discards a frame whose HLC
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
    /// # Violation posture
    ///
    /// Recovery never refuses to boot on the state it observes. Every input here
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
    /// unrecognised magic, or an unknown format version. A failed replay of an
    /// individual entry is NOT an error: it blocks the frontier and is logged.
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

            // `entries` is strictly ascending by WAL sequence: `Wal::unapplied`
            // sorts defensively (see its impl), and the loop below replays in that
            // order. Both the contiguous-frontier watermark logic AND the gate-free
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
            let mut frontier = 0u64;
            let mut first_failed: Option<u64> = None;
            let highest_enumerated = entries.iter().map(|e| e.sequence).max().unwrap_or(0);
            let now = current_millis();

            for entry in &entries {
                match Self::replay_entry(&inner_store, entry, now, self.replay_merge_gate).await {
                    Ok(()) => {
                        // Only a success that is still contiguous with the prefix
                        // may advance the frontier. Once an entry has failed, later
                        // successes are kept — re-running them next boot is free —
                        // but they must NOT license GC of the frame that failed.
                        //
                        // "Free" rests on a different argument per frame kind:
                        // Lww on the read-compare in `replay_entry`; a
                        // `WalOp::Store` OR snapshot on monotonicity; and a
                        // `WalOp::OrDelta` on set-algebra fold idempotency
                        // through `crdt::apply_or_delta`, over the whole
                        // un-applied window replayed in sequence order from the
                        // durable-store base. Monotonicity does not cover the
                        // delta kind.
                        if first_failed.is_none() {
                            frontier = entry.sequence;
                        }
                    }
                    Err(err) => {
                        // A store error here is environmental, so it is logged and
                        // the boot continues; refusing to boot would convert a
                        // transient backend failure into an availability outage.
                        tracing::warn!(
                            partition = partition_id,
                            map = %entry.map,
                            key = %entry.key,
                            seq = entry.sequence,
                            error = %err,
                            "WAL recovery: replay failed for entry; \
                             watermark stops below it and it stays replayable"
                        );
                        first_failed.get_or_insert(entry.sequence);
                    }
                }
            }

            // Harness-only seam: re-replay the oldest un-applied frame once more,
            // AFTER the in-order pass has left the newest value durable. This
            // reproduces the stale re-replay the live `flush_key` path can cause
            // (an older frame lingering in the replay window over a frameless newer
            // durable write) — a condition generated ops cannot manufacture because
            // in-order replay always ends on the newest frame. With the merge gate
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

            // Marking `0` would be a no-op on the sidecar but would still run
            // seal/rotate and GC for their own side effects; nothing was replayed,
            // so skip the call and its I/O entirely.
            if frontier == 0 {
                continue;
            }

            // Mark the contiguous prefix applied so a clean restart is a no-op.
            if let Err(err) = self.wal.mark_applied(partition_id, frontier).await {
                tracing::warn!(
                    partition = partition_id,
                    frontier,
                    error = %err,
                    "WAL recovery: failed to mark entries applied; \
                     next restart will re-replay (safe but redundant)"
                );
            }
        }

        Ok(())
    }
}

/// The recovery fold: ONE delta, folded onto the key's CURRENT durable value.
///
/// `WalRecovery` is the implementor because it is already the object that owns
/// replay — a separate fold type would be a second seam over the same algebra.
/// The receiver is dropped because the fold is a pure function of
/// `(base, delta)`: `replay_entry` is an associated function reached as
/// `WalRecovery::replay_entry(inner_store, entry, now, merge_gate)` and holds no
/// `WalRecovery` value to call a method on.
///
/// The fold owns no algebra of its own. It normalizes the base through the live
/// `crdt::normalize_to_or_map` and applies through the live
/// `crdt::apply_or_delta` — the same two functions the runtime OR write path
/// calls, in the same order (TG-OR-003). A second hand-written copy of either
/// would be free to drift from what the write path actually did, and on this
/// path drift is a silently lost mutation rather than a visible error.
impl OrDeltaFold for WalRecovery {
    /// One frame, one call: the caller passes the store's value as `base` and
    /// writes the result straight back, so the DURABLE STORE is the accumulator.
    /// The parameter is a single delta, not a window, so a key's frames cannot be
    /// collected out of the replay stream and folded as a batch.
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
/// and the `.applied` sidecar helpers are private, and sibling test modules
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

    /// Reads the partition's applied watermark from the `.applied` sidecar FILE.
    ///
    /// The value is durable by construction — it comes off disk, not from an
    /// in-memory mirror that would let a test pass on state a restart would lose.
    pub(crate) fn test_read_applied_sequence(&self, partition: u32) -> u64 {
        Self::read_applied_sequence(&self.applied_path(partition))
    }

    /// Forces the pre-unlink watermark re-read to return `value`, or restores the
    /// real sidecar read with `None`.
    ///
    /// The re-validation this drives is unreachable in production — the sidecar
    /// clamps monotonically, so a re-read can only return a value at or above the
    /// watermark GC already computed — so the path exists to be forced, not to be
    /// provoked by contriving a regression that cannot happen.
    pub(crate) fn test_force_gc_revalidation_watermark(&self, value: Option<u64>) {
        *self
            .gc_revalidation_override
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
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

    /// Sets the TG-WAL-003 crash-point knob (R7) `mark_applied` reads on its
    /// next call. `GcCrashPoint::None` restores normal behaviour.
    pub(crate) fn test_set_gc_crash_point(&self, point: GcCrashPoint) {
        *self
            .gc_crash_point
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = point;
    }

    /// Sets the TG-WAL-003 order-inversion knob (R6) `mark_applied` reads on
    /// its next call. `GcOrderMode::FsyncThenUnlink` restores the production
    /// order.
    pub(crate) fn test_set_gc_order_mode(&self, mode: GcOrderMode) {
        *self
            .gc_order_mode
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = mode;
    }
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
                    "if first_failed.is_none()",
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

    /// No file in this repository asserts that no emitter exists.
    ///
    /// Every count runs on the WHITESPACE-NORMALIZED view, never on raw lines. A
    /// bare single-line search does not satisfy this: the two claims that started
    /// this lineage were both line-wrapped inside doc comments, so a raw search
    /// for them returned zero before a single character had been changed. That
    /// false green is the exact defect class this scan replaces.
    #[test]
    fn no_document_still_claims_that_nothing_emits_a_delta_frame() {
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

        let crdt = normalized(CRDT_RS);
        assert!(
            crdt.contains("assert_eq!( total, 3,") || crdt.contains("total, 3,"),
            "the store-side implementor scan must count THREE -- the defaulted definition, \
             the one production override, and the one test spy"
        );
        assert!(
            crdt.contains("total, 2,"),
            "the demand-predicate scan counts a DIFFERENT symbol this spec adds no \
             implementor of, so its count must NOT have moved with the store-side one"
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
        WalEntry {
            map: "m".to_string(),
            key: key.to_string(),
            op: WalOp::OrDelta { delta },
            timestamp: None,
            sequence: 1,
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
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

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

        let store = Arc::new(MergeProbeStore::default());
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
            let wal = WalWriter::new(dir.path().to_path_buf(), policy).unwrap();
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
    async fn forced_watermark_regression_retains_the_sealed_segment() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        // Seals frames 1..=3 into one sealed segment and licenses its reclamation.
        // The forced re-read lands below the segment's highest sequence, which the
        // monotonic sidecar can never do on its own.
        let sealed_path = dir.path().join(segment::format_segment_filename(0, 0));
        wal.test_force_gc_revalidation_watermark(Some(0));
        wal.mark_applied(0, 3).await.unwrap();

        assert!(
            sealed_path.exists(),
            "a segment the re-read watermark does not cover must be retained, not unlinked"
        );

        // Restoring the real read lets the next pass reclaim it: the retention is a
        // deferral, never a permanent leak.
        wal.test_force_gc_revalidation_watermark(None);
        wal.mark_applied(0, 3).await.unwrap();
        assert!(
            !sealed_path.exists(),
            "GC must resume once the re-read watermark covers the segment again"
        );
    }

    // -----------------------------------------------------------------------
    // TG-WAL-003 seams (R6/R7): GcCrashPoint and GcOrderMode wiring
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn gc_crash_point_pre_unlink_ends_the_call_before_reclaiming() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        let sealed_path = dir.path().join(segment::format_segment_filename(0, 0));

        wal.test_set_gc_crash_point(GcCrashPoint::PreUnlink);
        wal.mark_applied(0, 3).await.unwrap();

        // The sidecar advance happens before the injected crash point, so it
        // must be durable even though the call returned early.
        assert_eq!(
            wal.test_read_applied_sequence(0),
            3,
            "the watermark write+fsync precedes the crash point and must be durable"
        );
        // The unlink loop is after the injected crash point, so the sealed
        // segment must still be on disk.
        assert!(
            sealed_path.exists(),
            "PreUnlink must end the call before the sealed segment is unlinked"
        );

        // Restoring normal behaviour lets GC resume and finish the deferred
        // reclamation on the next call — nothing was permanently lost.
        wal.test_set_gc_crash_point(GcCrashPoint::None);
        wal.mark_applied(0, 3).await.unwrap();
        assert!(
            !sealed_path.exists(),
            "GC must complete the deferred reclamation once the crash point is cleared"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gc_order_mode_unlink_then_fsync_bypasses_the_durable_revalidation() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        let sealed_path = dir.path().join(segment::format_segment_filename(0, 0));

        // Force a stale re-read that would RETAIN the segment under the
        // production (FsyncThenUnlink) order — proven above by
        // `forced_watermark_regression_retains_the_sealed_segment`. The
        // order-inversion seam has no durable sidecar to re-read at its point
        // in the (inverted) sequence, so it must reclaim against the
        // in-memory watermark and ignore this override entirely — that
        // divergence is exactly the TG-WAL-003 hazard being reintroduced.
        wal.test_force_gc_revalidation_watermark(Some(0));
        wal.test_set_gc_order_mode(GcOrderMode::UnlinkThenFsync);
        wal.mark_applied(0, 3).await.unwrap();

        assert!(
            !sealed_path.exists(),
            "UnlinkThenFsync must reclaim against the in-memory watermark, \
             not the (irrelevant, not-yet-durable) forced re-read"
        );
        assert_eq!(
            wal.test_read_applied_sequence(0),
            3,
            "the sidecar is still made durable by the end of the call, just \
             after the unlink instead of before it"
        );

        wal.test_force_gc_revalidation_watermark(None);
        wal.test_set_gc_order_mode(GcOrderMode::FsyncThenUnlink);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gc_order_mode_unlink_then_fsync_plus_crash_point_leaves_stale_sidecar() {
        // AC7(b): the genuine TG-WAL-003 loss window only exists BETWEEN the
        // wrongly-early unlink and the deferred sidecar write. A crash the
        // harness fires only between whole `mark_applied` calls would never
        // observe it, because a *completed* inverted-order call still leaves
        // both steps durable by the time it returns (proven by the sibling
        // test above: segment gone AND sidecar at 3). This test proves the
        // combination of `UnlinkThenFsync` + `GcCrashPoint::PreUnlink` lands
        // the crash inside that window instead: segment gone, sidecar STILL
        // at its old value.
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        let sealed_path = dir.path().join(segment::format_segment_filename(0, 0));
        assert_eq!(
            wal.test_read_applied_sequence(0),
            0,
            "no watermark has been recorded yet"
        );

        wal.test_set_gc_order_mode(GcOrderMode::UnlinkThenFsync);
        wal.test_set_gc_crash_point(GcCrashPoint::PreUnlink);
        wal.mark_applied(0, 3).await.unwrap();

        assert!(
            !sealed_path.exists(),
            "the inverted order unlinks BEFORE the deferred sidecar write, so the \
             segment must already be gone at the (post-unlink) crash point"
        );
        assert_eq!(
            wal.test_read_applied_sequence(0),
            0,
            "the crash point fires before the deferred sidecar write, so the \
             sidecar must still be at its OLD value — the exact TG-WAL-003 \
             hazard: a restart seeded from this stale watermark under-seeds \
             max_observed_sequence for the now-vanished segment's frames"
        );

        // Contrast: the production order under the SAME crash point leaves
        // the sidecar durable and the segment retained (proven by
        // `gc_crash_point_pre_unlink_ends_the_call_before_reclaiming` above)
        // — the inversion, not the crash point alone, is what produces the
        // inconsistent on-disk state.
        wal.test_set_gc_order_mode(GcOrderMode::FsyncThenUnlink);
        wal.test_set_gc_crash_point(GcCrashPoint::None);
    }

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
    async fn a_flush_key_only_terminal_advances_the_durable_sidecar() {
        use crate::storage::datastores::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};

        // `partition_for` is private to the write-behind module, so the partition
        // is located by the sidecar that MOVED rather than by recomputing the
        // hash — which also makes the assertion independent of the hash function.
        fn advanced_partitions(wal: &WalWriter) -> Vec<u32> {
            (0..topgun_core::PARTITION_COUNT)
                .filter(|p| wal.test_read_applied_sequence(*p) >= 1)
                .collect()
        }

        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::PerOp).unwrap();
        let inner: Arc<dyn MapDataStore> = Arc::new(ReplayStore::default());

        // Nothing may drain on its own: `flush_key` must be the ONLY possible
        // source of the advance, or the flush loop could satisfy the assertion.
        let config = WriteBehindConfig {
            write_delay_ms: 600_000,
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
            advanced_partitions(&wal).is_empty(),
            "a buffered write must not have advanced anything yet"
        );

        store.flush_key("m", "k", &value, false).await.unwrap();

        // Read off the `.applied` FILE: an implementation that resolves the
        // carried sequences but never calls `mark_applied` moves the pending set
        // correctly and grows the WAL forever, and only the durable read sees it.
        assert_eq!(
            advanced_partitions(&wal).len(),
            1,
            "a flush_key Ok terminal must resolve AND advance the durable sidecar"
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
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        let before = skipped_total(&observability.render_metrics());

        for seq in 1..=3u64 {
            wal.append(0, &make_wal_entry(seq)).await.unwrap();
        }
        wal.test_force_gc_revalidation_watermark(Some(0));
        wal.mark_applied(0, 3).await.unwrap();
        wal.test_force_gc_revalidation_watermark(None);

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
        let wal = Arc::new(WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap());

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

    #[tokio::test]
    async fn applied_sidecar_is_readable_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let wal = WalWriter::new(dir.path().to_path_buf(), WalFsyncPolicy::None).unwrap();

        assert_eq!(wal.test_read_applied_sequence(0), 0);

        wal.append(0, &make_wal_entry(1)).await.unwrap();
        wal.mark_applied(0, 1).await.unwrap();

        assert_eq!(
            wal.test_read_applied_sequence(0),
            1,
            "The accessor must observe the durable sidecar value, not an in-memory mirror"
        );
    }
}
