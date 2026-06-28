//! Per-partition WAL segment abstraction (active-vs-sealed boundary).
//!
//! A partition's Write-Ahead Log is split into a sequence of fixed-lineage
//! **segment** files instead of one ever-growing file. Appends always target the
//! single **active** segment; compaction **seals** the active segment, starts a
//! fresh active segment, and **deletes** sealed segments whose entire sequence
//! range is at or below the applied watermark. This replaces the O(n) full-file
//! read→re-encode→rename compaction with a single `unlink` per reclaimed segment,
//! and lets appenders run without contending on the sealed set.
//!
//! Segment metadata lives entirely in the **filename** (`partition-{id:03}-
//! {first_seq:020}.log`), so recovery can discover and order a partition's
//! segments — and GC can read a sealed segment's coverage — without opening the
//! file or parsing an in-file header. The frame codec in `format.rs` is unchanged:
//! each segment file is a plain stream of the same frames a single-file WAL wrote.
//!
//! This module is the type/trait surface the `WalWriter` rewrite builds on. The
//! method bodies that the writer owns (the actual seal/rotate/GC I/O) are stubbed
//! here so the surface can be reviewed before the implementation lands.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

// ---------------------------------------------------------------------------
// Filename convention
// ---------------------------------------------------------------------------

/// Filename prefix shared by every segment of every partition.
///
/// Mirrors the single-file WAL naming (`partition-{id:03}.log`) so a reader can
/// still tell a WAL file apart from the `.applied` sidecar at a glance.
pub const SEGMENT_FILE_PREFIX: &str = "partition-";

/// Filename suffix for a segment file.
///
/// Kept identical to the single-file WAL's `.log` extension so the corruption
/// taxonomy, tooling, and operator expectations carry over unchanged; the
/// segment distinction is encoded in the embedded `first_seq`, not the extension.
pub const SEGMENT_FILE_SUFFIX: &str = ".log";

/// Zero-padding width for the partition id in a segment filename.
///
/// Matches the existing `partition-{id:03}` convention so segment files sort
/// next to the legacy single-file layout in a directory listing.
pub const SEGMENT_PARTITION_PAD: usize = 3;

/// Zero-padding width for the `first_seq` component of a segment filename.
///
/// `u64::MAX` is 20 decimal digits, so padding to 20 makes a partition's segments
/// sort lexicographically in ascending `first_seq` order — recovery can therefore
/// order segments by filename alone, without opening any of them.
pub const SEGMENT_SEQ_PAD: usize = 20;

/// Formats the on-disk filename for a segment.
///
/// The returned name encodes both the owning partition and the segment's
/// `first_seq` (its lowest covered sequence), e.g. `partition-005-
/// 00000000000000000042.log`. Recovery and GC parse this with
/// [`parse_segment_filename`]; neither needs to open the file to learn its
/// partition or coverage start.
#[must_use]
pub fn format_segment_filename(partition: u32, first_seq: u64) -> String {
    let pad_p = SEGMENT_PARTITION_PAD;
    let pad_s = SEGMENT_SEQ_PAD;
    format!("{SEGMENT_FILE_PREFIX}{partition:0pad_p$}-{first_seq:0pad_s$}{SEGMENT_FILE_SUFFIX}")
}

/// Parses a segment filename back into its `(partition, first_seq)` components.
///
/// Returns `None` for any name that is not a segment file (wrong prefix/suffix,
/// missing separator, or non-numeric components) so callers can skip sidecar and
/// unrelated files during directory discovery without erroring.
#[must_use]
pub fn parse_segment_filename(name: &str) -> Option<(u32, u64)> {
    let inner = name
        .strip_prefix(SEGMENT_FILE_PREFIX)?
        .strip_suffix(SEGMENT_FILE_SUFFIX)?;
    // The first '-' separates the partition id from the first_seq. first_seq is
    // pure digits, so splitting on the first '-' is unambiguous.
    let (partition_str, first_seq_str) = inner.split_once('-')?;
    let partition = partition_str.parse::<u32>().ok()?;
    let first_seq = first_seq_str.parse::<u64>().ok()?;
    Some((partition, first_seq))
}

// ---------------------------------------------------------------------------
// SegmentState — active vs sealed
// ---------------------------------------------------------------------------

/// Whether a [`Segment`] is still being appended to or has been frozen.
///
/// The active segment owns an open append file handle; a sealed segment is
/// immutable and may carry no handle (it is reopened read-only on demand). GC
/// only ever deletes sealed segments — the variant is the structural guard that
/// the active segment is never reclaimed.
pub enum SegmentState {
    /// The single segment appends currently target. Holds the open append handle
    /// and the live batched-fsync accounting.
    Active {
        /// Open append handle for the active segment file.
        file: tokio::fs::File,
        /// Frames written to this handle since the last fsync, carried over from
        /// the single-file batched-fsync accounting (`u32`, a non-negative count).
        unfsynced_count: u32,
    },
    /// A frozen segment. No further frames are appended; it is reopened read-only
    /// for recovery/`unapplied` and dropped by GC with a single `unlink`.
    Sealed,
}

// ---------------------------------------------------------------------------
// Segment — one on-disk segment file
// ---------------------------------------------------------------------------

/// One on-disk WAL segment file for a single partition.
///
/// A segment covers a contiguous-by-arrival range of sequence numbers
/// `[first_seq, max_seq]`. `first_seq` is fixed at creation (and encoded in the
/// filename); `max_seq` advances as frames are appended to the active segment and
/// is frozen when the segment is sealed. Sequences are `u64`; the fsync counter is
/// `u32` — no `f64` (per the project type-mapping rules).
pub struct Segment {
    /// Absolute path to this segment's file on disk.
    path: PathBuf,
    /// Active (open handle + fsync accounting) or sealed (immutable) state.
    state: SegmentState,
    /// Lowest sequence number this segment covers. Fixed at creation and mirrored
    /// in the filename so coverage is known without opening the file.
    first_seq: u64,
    /// Highest sequence number written to this segment so far. Advances on append
    /// to the active segment; frozen at seal time. A freshly created empty segment
    /// seeds this to `first_seq` as a floor — `max_seq_explicit` distinguishes that
    /// seed from a genuine single-frame segment whose only frame is `first_seq`.
    max_seq: u64,
    /// Whether `max_seq` reflects a frame that was actually appended (or decoded),
    /// rather than the `first_seq` floor seeded into an empty active segment. Guards
    /// the empty-segment +1 overcount from leaking into watermark / GC decisions.
    max_seq_explicit: bool,
}

impl Segment {
    /// Creates an active segment rooted at `path` covering sequences from
    /// `first_seq`, taking ownership of an already-open append `file` handle.
    ///
    /// The caller (the writer) is responsible for having opened `path` in
    /// create+append mode; this constructor only wires the bookkeeping. `max_seq`
    /// is seeded to `first_seq` and advances as frames are appended.
    #[must_use]
    pub fn new_active(path: PathBuf, first_seq: u64, file: tokio::fs::File) -> Self {
        Self {
            path,
            state: SegmentState::Active {
                file,
                unfsynced_count: 0,
            },
            first_seq,
            max_seq: first_seq,
            max_seq_explicit: false,
        }
    }

    /// Constructs a sealed-segment handle for an already-existing on-disk file
    /// discovered during recovery, with its known sequence coverage.
    ///
    /// `first_seq` comes from the filename; `max_seq` is the highest sequence the
    /// caller decoded from the file (or computed from the filename of the next
    /// segment). No file handle is held — sealed segments are reopened read-only
    /// on demand.
    #[must_use]
    pub fn sealed_existing(path: PathBuf, first_seq: u64, max_seq: u64) -> Self {
        Self {
            path,
            state: SegmentState::Sealed,
            first_seq,
            max_seq,
            // A sealed segment discovered on disk covers real decoded frames; its
            // max_seq is authoritative even when it equals first_seq.
            max_seq_explicit: true,
        }
    }

    /// Path to this segment's file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Lowest sequence number this segment covers (fixed at creation).
    #[must_use]
    pub fn first_seq(&self) -> u64 {
        self.first_seq
    }

    /// Highest sequence number written to this segment so far.
    #[must_use]
    pub fn max_seq(&self) -> u64 {
        self.max_seq
    }

    /// Whether this segment is the active (append-target) segment.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self.state, SegmentState::Active { .. })
    }

    /// Whether this segment is sealed (immutable, GC-eligible).
    #[must_use]
    pub fn is_sealed(&self) -> bool {
        matches!(self.state, SegmentState::Sealed)
    }

    /// Whether this segment has actually had any frame appended to it.
    ///
    /// A freshly-created active segment seeds `max_seq = first_seq`, which would
    /// otherwise overstate its coverage by one phantom sequence. Callers use this
    /// to treat an empty segment as covering no frames (so its seeded `max_seq`
    /// never leaks into a watermark / GC decision).
    #[must_use]
    pub fn has_frames(&self) -> bool {
        self.max_seq > self.first_seq || self.max_seq_explicit
    }

    /// Appends an already-encoded frame to the active segment, advancing
    /// `max_seq` to `sequence` and bumping the un-fsynced frame counter.
    ///
    /// Performs the OS-visibility flush (not an fsync) so a concurrent fresh
    /// reader observes the bytes; durability is governed by the caller's fsync
    /// policy. Returns the new un-fsynced count so the caller can apply its
    /// batched-fsync threshold.
    ///
    /// # Errors
    ///
    /// Returns an error if the segment is sealed (a programming bug) or if the
    /// underlying write/flush fails.
    pub async fn append_frame(&mut self, frame: &[u8], sequence: u64) -> anyhow::Result<u32> {
        let path = self.path.clone();
        let SegmentState::Active {
            file,
            unfsynced_count,
        } = &mut self.state
        else {
            anyhow::bail!("append to sealed WAL segment {}", path.display());
        };
        file.write_all(frame)
            .await
            .map_err(|e| anyhow::anyhow!("WAL write failed for {}: {e}", path.display()))?;
        // Push tokio's buffer to the OS so a concurrent fresh read of this
        // segment observes the frame. This is visibility, not durability.
        file.flush()
            .await
            .map_err(|e| anyhow::anyhow!("WAL flush failed for {}: {e}", path.display()))?;
        *unfsynced_count = unfsynced_count.saturating_add(1);
        let count = *unfsynced_count;
        // A frame was written; coverage now genuinely reaches `sequence`.
        self.max_seq = sequence;
        self.max_seq_explicit = true;
        Ok(count)
    }

    /// fsyncs the active segment's data and resets the un-fsynced counter.
    ///
    /// # Errors
    ///
    /// Returns an error if the segment is sealed or the fsync fails.
    pub async fn sync_data(&mut self) -> anyhow::Result<()> {
        let path = self.path.clone();
        let SegmentState::Active {
            file,
            unfsynced_count,
        } = &mut self.state
        else {
            anyhow::bail!("fsync of sealed WAL segment {}", path.display());
        };
        file.sync_data()
            .await
            .map_err(|e| anyhow::anyhow!("WAL fsync failed for {}: {e}", path.display()))?;
        *unfsynced_count = 0;
        Ok(())
    }

    /// fsyncs the active segment's data, then freezes it into a sealed segment.
    ///
    /// The data fsync happens **before** the segment is frozen because a sealed
    /// segment is non-last in the live ordering, and a torn tail on a non-last
    /// segment is fatal during recovery (truncated-tail tolerance is active-only).
    /// Consumes the open file handle (dropping it closes the descriptor).
    ///
    /// # Errors
    ///
    /// Returns an error if the segment is already sealed or the fsync fails.
    pub async fn seal(self) -> anyhow::Result<Self> {
        let Segment {
            path,
            state,
            first_seq,
            max_seq,
            max_seq_explicit,
        } = self;
        let SegmentState::Active { file, .. } = state else {
            anyhow::bail!("seal of already-sealed WAL segment {}", path.display());
        };
        file.sync_data()
            .await
            .map_err(|e| anyhow::anyhow!("WAL seal fsync failed for {}: {e}", path.display()))?;
        drop(file);
        Ok(Self {
            path,
            state: SegmentState::Sealed,
            first_seq,
            max_seq,
            max_seq_explicit,
        })
    }

    /// Seeds an active segment reopened during recovery with the `max_seq` decoded
    /// from its existing frames, marking the coverage as explicit (real frames).
    ///
    /// `max_seq` is only raised, never lowered — a recovered file already holds
    /// those frames, so its coverage cannot retreat. A value at or below
    /// `first_seq` means the file held no frames, leaving the empty floor intact.
    pub fn set_recovered_max_seq(&mut self, max_seq: u64) {
        if max_seq > self.first_seq {
            self.max_seq = max_seq;
            self.max_seq_explicit = true;
        }
    }

    /// Number of frames written since the last fsync (0 for a sealed segment).
    #[must_use]
    pub fn unfsynced_count(&self) -> u32 {
        match &self.state {
            SegmentState::Active {
                unfsynced_count, ..
            } => *unfsynced_count,
            SegmentState::Sealed => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// SegmentSet — per-partition active + sealed segments
// ---------------------------------------------------------------------------

/// All segments of a single partition: one active segment plus an ordered list of
/// sealed segments.
///
/// Invariants the writer must uphold (enforced by the methods below in G2):
/// - exactly one active segment exists at any time;
/// - sealed segments are ordered by ascending `first_seq` (so recovery and
///   `unapplied` can read them in sequence order without re-sorting);
/// - every sealed segment's range is strictly below the active segment's
///   `first_seq` (sealing freezes the boundary before a fresh active starts);
/// - GC only ever removes from `sealed` — the active segment is structurally
///   unreachable from `gc_below_watermark`.
pub struct SegmentSet {
    /// The partition this set belongs to (mirrored in every segment filename).
    partition: u32,
    /// Sealed, immutable segments in ascending `first_seq` order. The front is the
    /// oldest; GC reclaims from the front up to the watermark.
    sealed: Vec<Segment>,
    /// The single segment that appends currently target.
    active: Segment,
}

impl SegmentSet {
    /// Creates a `SegmentSet` for `partition` with `active` as its sole segment
    /// and no sealed segments (a fresh partition).
    #[must_use]
    pub fn new(partition: u32, active: Segment) -> Self {
        Self {
            partition,
            sealed: Vec::new(),
            active,
        }
    }

    /// Reconstructs a `SegmentSet` from segments discovered on disk during
    /// recovery: the already-ordered `sealed` segments plus the `active` segment.
    ///
    /// `sealed` MUST be in ascending `first_seq` order and all below
    /// `active.first_seq` — recovery builds it that way from the filename ordering.
    #[must_use]
    pub fn from_recovered(partition: u32, sealed: Vec<Segment>, active: Segment) -> Self {
        Self {
            partition,
            sealed,
            active,
        }
    }

    /// The partition this set serves.
    #[must_use]
    pub fn partition(&self) -> u32 {
        self.partition
    }

    /// The active segment — the append target.
    ///
    /// `append` writes here; this is the only segment a writer mutates outside of
    /// seal/rotate. Returned mutably so the writer can advance `max_seq` and the
    /// fsync accounting on append.
    pub fn append_target(&mut self) -> &mut Segment {
        &mut self.active
    }

    /// Seals the current active segment and rotates a fresh active segment in.
    ///
    /// Ordering is correctness-critical (the RULE): the current active segment is
    /// frozen into the sealed set *before* the new active is wired in, so no window
    /// exists in which an already-acked frame is neither in a sealed segment nor in
    /// the active segment. `next_first_seq` is the `first_seq` of the new active
    /// segment (`current active.max_seq + 1`), and `new_active_file` is its
    /// already-opened append handle.
    ///
    /// The body (the file open + handle swap + sealed push) is owned by the writer
    /// and lands with the G2 implementation; this G1 stub freezes `max_seq` and
    /// installs the new active segment so the type compiles and the surface is
    /// reviewable, but the full crash-ordered I/O path is G2's.
    pub fn seal_active_and_rotate(
        &mut self,
        next_first_seq: u64,
        new_active_file: tokio::fs::File,
    ) {
        let sealed_path = self.active.path().to_path_buf();
        let sealed_first = self.active.first_seq();
        let sealed_max = self.active.max_seq();
        let new_path = self
            .active
            .path()
            .with_file_name(format_segment_filename(self.partition, next_first_seq));

        let frozen = Segment::sealed_existing(sealed_path, sealed_first, sealed_max);
        // Freeze the current active into the sealed set BEFORE wiring the fresh
        // active in, so no window exists where an acked frame belongs to neither.
        self.sealed.push(frozen);
        self.active = Segment::new_active(new_path, next_first_seq, new_active_file);
    }

    /// Selects and deletes sealed segments fully covered by the applied watermark.
    ///
    /// A sealed segment is reclaimable iff `max_seq <= applied_through` — every
    /// frame it holds has been durably applied to the inner store, so dropping the
    /// file loses nothing. The active segment is NEVER considered, regardless of
    /// its `max_seq`: appenders may still be writing to it, so it stays until a
    /// future seal moves its frames into the sealed set. Reclaimed segments are
    /// removed from the front of `sealed` (oldest first) and `unlink`ed.
    ///
    /// Returns the paths that were deleted so the caller can log/account for them.
    /// The actual `unlink` I/O is owned by the writer and lands with G2; this G1
    /// stub performs the watermark **selection** (never touching `active`) so the
    /// surface and the active-segment-exclusion invariant are reviewable, and
    /// drains the selected handles from the sealed set.
    ///
    /// # Errors
    ///
    /// Returns an error if a sealed segment file cannot be unlinked (G2).
    pub fn gc_below_watermark(&mut self, applied_through: u64) -> anyhow::Result<Vec<PathBuf>> {
        // Reclaimable sealed segments are a prefix of the ascending-ordered set
        // (oldest first). The active segment is intentionally not in `sealed`, so
        // it can never be selected here regardless of its `max_seq`.
        let reclaim = self
            .sealed
            .iter()
            .take_while(|s| s.max_seq() <= applied_through)
            .count();
        let drained: Vec<PathBuf> = self
            .sealed
            .drain(..reclaim)
            .map(|s| s.path().to_path_buf())
            .collect();
        Ok(drained)
    }

    /// Every live segment in ascending sequence order: sealed segments (oldest
    /// first) followed by the active segment last.
    ///
    /// Recovery and `unapplied` iterate this to decode frames across the whole
    /// partition; the active segment is last so a `TruncatedTail` is only ever
    /// tolerated on it (a mid-stream truncation in a sealed segment is corruption).
    #[must_use]
    pub fn all_segments_in_seq_order(&self) -> Vec<&Segment> {
        let mut out: Vec<&Segment> = self.sealed.iter().collect();
        out.push(&self.active);
        out
    }
}
