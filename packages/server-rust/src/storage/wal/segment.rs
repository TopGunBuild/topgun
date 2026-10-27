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
//! The **filename** (`partition-{id:03}-{first_seq:020}.log`) names a segment's
//! partition and lowest sequence, so recovery can discover and order a
//! partition's segments without opening them. Every segment this binary creates
//! also starts with a [`SegmentHeader`] that repeats both and carries a fresh
//! random salt (`TG-ENC-003`): two files that ever carry the same name are told
//! apart by their salt. A file without a header is a segment of an older binary,
//! a plain stream of v1 frames, read and never appended to.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

// ---------------------------------------------------------------------------
// Segment header
// ---------------------------------------------------------------------------

/// Magic of a segment header: ASCII "TGWS". A segment file is v2 iff it starts
/// with it, v1 iff it starts with the frame magic "TGWL".
pub const SEGMENT_MAGIC: u32 = 0x54_47_57_53;

/// The only header version this binary reads or writes.
pub const SEGMENT_HEADER_VERSION: u8 = 2;

/// Header bytes the CRC32C covers: magic through `first_seq`.
const SEGMENT_HEADER_BODY_LEN: usize = 57;

/// Total bytes of a segment header: its body and a CRC32C u32 BE.
pub const SEGMENT_HEADER_LEN: usize = SEGMENT_HEADER_BODY_LEN + 4;

/// How a frame is assigned to a WAL partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentLayout {
    /// `fnv1a("{map}:{key}") % 271`.
    Fnv1aMod271 = 0,
}

/// The schema of a frame's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadSchema {
    /// The msgpack `WalEntry` this binary reads and writes.
    WalEntryV1 = 1,
}

/// How a segment's frame payloads are protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentCipher {
    /// Payloads are written as encoded.
    None = 0,
    /// Payloads are sealed with XChaCha20-Poly1305 under a key derived from
    /// the header; reading them needs the key named by `key_id`.
    XChaCha20Poly1305 = 1,
}

/// The header every segment this binary creates starts with:
/// `MAGIC u32 ‖ header_version u8 ‖ layout u8 ‖ payload_schema u16 BE ‖
/// cipher u8 ‖ key_id u32 BE ‖ salt [32] ‖ partition u32 BE ‖ first_seq u64 BE ‖
/// CRC32C u32 BE`, 61 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentHeader {
    pub layout: SegmentLayout,
    pub payload_schema: PayloadSchema,
    pub cipher: SegmentCipher,
    /// The data key the segment is sealed with; 0 iff `cipher` is `None`.
    pub key_id: u32,
    /// Random per created file, so no two files share an identity.
    pub salt: [u8; 32],
    pub partition: u32,
    pub first_seq: u64,
}

/// Why a segment header was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderError {
    /// The CRC32C does not match the header body.
    BadCrc {
        stored: u32,
        computed: u32,
    },
    UnknownVersion(u8),
    UnknownLayout(u8),
    UnknownPayloadSchema(u16),
    UnknownCipher(u8),
    /// `key_id` is 0 with a cipher, or not 0 without one.
    KeyIdMismatch {
        cipher: SegmentCipher,
        key_id: u32,
    },
}

impl std::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadCrc { stored, computed } => write!(
                f,
                "header CRC {stored:#010x} does not match its content ({computed:#010x})"
            ),
            Self::UnknownVersion(v) => write!(f, "unknown header version {v}"),
            Self::UnknownLayout(v) => write!(f, "unknown partition layout {v}"),
            Self::UnknownPayloadSchema(v) => write!(f, "unknown payload schema {v}"),
            Self::UnknownCipher(v) => write!(f, "unknown cipher {v}"),
            Self::KeyIdMismatch { cipher, key_id } => {
                write!(f, "key id {key_id} does not fit cipher {cipher:?}")
            }
        }
    }
}

impl SegmentHeader {
    /// An unsealed header for a new segment of `partition` starting at
    /// `first_seq`, with a fresh random salt.
    #[must_use]
    pub fn new_unsealed(partition: u32, first_seq: u64) -> Self {
        Self {
            layout: SegmentLayout::Fnv1aMod271,
            payload_schema: PayloadSchema::WalEntryV1,
            cipher: SegmentCipher::None,
            key_id: 0,
            salt: rand::random(),
            partition,
            first_seq,
        }
    }

    /// The header's 61 bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; SEGMENT_HEADER_LEN] {
        let mut out = [0u8; SEGMENT_HEADER_LEN];
        out[0..4].copy_from_slice(&SEGMENT_MAGIC.to_be_bytes());
        out[4] = SEGMENT_HEADER_VERSION;
        out[5] = self.layout as u8;
        out[6..8].copy_from_slice(&(self.payload_schema as u16).to_be_bytes());
        out[8] = self.cipher as u8;
        out[9..13].copy_from_slice(&self.key_id.to_be_bytes());
        out[13..45].copy_from_slice(&self.salt);
        out[45..49].copy_from_slice(&self.partition.to_be_bytes());
        out[49..57].copy_from_slice(&self.first_seq.to_be_bytes());
        let crc = crc32c::crc32c(&out[..SEGMENT_HEADER_BODY_LEN]);
        out[SEGMENT_HEADER_BODY_LEN..].copy_from_slice(&crc.to_be_bytes());
        out
    }

    /// Parses the header at the start of `bytes`, which must hold at least
    /// [`SEGMENT_HEADER_LEN`] bytes and start with [`SEGMENT_MAGIC`]; the caller
    /// classifies a shorter file.
    ///
    /// # Errors
    ///
    /// A CRC mismatch or a field value this binary does not know.
    ///
    /// # Panics
    ///
    /// Never for input of at least [`SEGMENT_HEADER_LEN`] bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, HeaderError> {
        let h = &bytes[..SEGMENT_HEADER_LEN];
        let be32 =
            |r: std::ops::Range<usize>| u32::from_be_bytes(h[r].try_into().expect("4 bytes"));
        let stored = be32(SEGMENT_HEADER_BODY_LEN..SEGMENT_HEADER_LEN);
        let computed = crc32c::crc32c(&h[..SEGMENT_HEADER_BODY_LEN]);
        if stored != computed {
            return Err(HeaderError::BadCrc { stored, computed });
        }
        if h[4] != SEGMENT_HEADER_VERSION {
            return Err(HeaderError::UnknownVersion(h[4]));
        }
        let layout = match h[5] {
            0 => SegmentLayout::Fnv1aMod271,
            other => return Err(HeaderError::UnknownLayout(other)),
        };
        let payload_schema = match u16::from_be_bytes([h[6], h[7]]) {
            1 => PayloadSchema::WalEntryV1,
            other => return Err(HeaderError::UnknownPayloadSchema(other)),
        };
        let cipher = match h[8] {
            0 => SegmentCipher::None,
            1 => SegmentCipher::XChaCha20Poly1305,
            other => return Err(HeaderError::UnknownCipher(other)),
        };
        let key_id = be32(9..13);
        if (cipher == SegmentCipher::None) != (key_id == 0) {
            return Err(HeaderError::KeyIdMismatch { cipher, key_id });
        }
        Ok(Self {
            layout,
            payload_schema,
            cipher,
            key_id,
            salt: h[13..45].try_into().expect("32 bytes"),
            partition: be32(45..49),
            first_seq: u64::from_be_bytes(h[49..57].try_into().expect("8 bytes")),
        })
    }
}

/// Creates the segment file at `path` holding only `header`, durably, before
/// any frame can be appended to it (`TG-ENC-003`, `TG-ENC-005`): the header is
/// written to `{path}.tmp` and synced, the temporary file is renamed to `path`,
/// and the directory is synced. Returns the open handle, positioned after the
/// header.
///
/// The rename replaces a file already at `path` only when that file holds no
/// intact frame — one a crash or a failed rotation left behind; the caller
/// guarantees it is not the installed active segment. A file that holds a frame
/// is refused rather than replaced.
///
/// # Errors
///
/// An I/O error, or a file at `path` that holds a frame.
pub fn create_segment_file(path: &Path, header: &SegmentHeader) -> anyhow::Result<std::fs::File> {
    use std::io::Write as _;
    match std::fs::read(path) {
        Ok(existing) => {
            if super::format::decode_segment(&existing).frames > 0 {
                anyhow::bail!(
                    "Cannot create WAL segment {}: a segment holding frames is already there",
                    path.display()
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => anyhow::bail!("Cannot read WAL segment {}: {e}", path.display()),
    }
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let mut file = std::fs::File::create(&tmp)
        .map_err(|e| anyhow::anyhow!("Cannot create WAL segment {}: {e}", tmp.display()))?;
    file.write_all(&header.encode())
        .map_err(|e| anyhow::anyhow!("Cannot write WAL segment {}: {e}", tmp.display()))?;
    file.sync_all()
        .map_err(|e| anyhow::anyhow!("Cannot fsync WAL segment {}: {e}", tmp.display()))?;
    #[cfg(test)]
    super::test_abort_if_armed(super::TestAbortPoint::CreateBeforeRename);
    std::fs::rename(&tmp, path).map_err(|e| {
        anyhow::anyhow!("Cannot rename {} to {}: {e}", tmp.display(), path.display())
    })?;
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("WAL segment {} has no directory", path.display()))?;
    super::fsync_dir(dir)?;
    Ok(file)
}

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
/// Returns `None` for any name that is not a segment file (wrong prefix/suffix
/// or non-numeric components) so callers can skip sidecar and unrelated files
/// during directory discovery without erroring.
///
/// A legacy single-file WAL name (`partition-NNN.log`, with no `first_seq`
/// component) is auto-migrated: it is treated as the partition's oldest segment
/// (`first_seq = 0`). Without this, a pre-rotation WAL file would be silently
/// skipped on upgrade and its un-applied frames lost. Discovery sorts by numeric
/// `first_seq`, so a legacy file (`first_seq = 0`) orders first.
#[must_use]
pub fn parse_segment_filename(name: &str) -> Option<(u32, u64)> {
    let inner = name
        .strip_prefix(SEGMENT_FILE_PREFIX)?
        .strip_suffix(SEGMENT_FILE_SUFFIX)?;
    // The first '-' separates the partition id from the first_seq. first_seq is
    // pure digits, so splitting on the first '-' is unambiguous.
    if let Some((partition_str, first_seq_str)) = inner.split_once('-') {
        let partition = partition_str.parse::<u32>().ok()?;
        let first_seq = first_seq_str.parse::<u64>().ok()?;
        Some((partition, first_seq))
    } else {
        // No separator → legacy single-file layout (`partition-NNN.log`). Treat
        // the whole inner string as the partition id at first_seq 0 so the file
        // is discovered and replayed instead of silently skipped on upgrade.
        let partition = inner.parse::<u32>().ok()?;
        Some((partition, 0))
    }
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
        /// The header this process wrote when it created the file.
        header: SegmentHeader,
        /// Index of the next frame in this segment: the count of frames framed
        /// before it. Taken only under the active-append lock (`TG-ENC-005`).
        next_index: u64,
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
    /// Creates an active segment rooted at `path`, which this process has just
    /// created with `header` ([`create_segment_file`]), taking ownership of its
    /// open `file` handle. `max_seq` is seeded to the header's `first_seq` and
    /// advances as frames are appended.
    #[must_use]
    pub fn new_active(path: PathBuf, header: SegmentHeader, file: tokio::fs::File) -> Self {
        Self {
            path,
            state: SegmentState::Active {
                file,
                unfsynced_count: 0,
                header,
                next_index: 0,
            },
            first_seq: header.first_seq,
            max_seq: header.first_seq,
            max_seq_explicit: false,
        }
    }

    /// The header of an active segment; `None` for a sealed one.
    #[must_use]
    pub fn header(&self) -> Option<&SegmentHeader> {
        match &self.state {
            SegmentState::Active { header, .. } => Some(header),
            SegmentState::Sealed => None,
        }
    }

    /// Takes the index of the frame about to be written to the active segment
    /// and advances the counter, so no index is handed out twice. Called under
    /// the active-append lock, after the frame's payload has encoded.
    ///
    /// # Errors
    ///
    /// The segment is sealed.
    pub fn take_frame_index(&mut self) -> anyhow::Result<u64> {
        let SegmentState::Active { next_index, .. } = &mut self.state else {
            anyhow::bail!("frame index of sealed WAL segment {}", self.path.display());
        };
        let index = *next_index;
        *next_index += 1;
        Ok(index)
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
            ..
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
            ..
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

    /// Freezes an active segment into a sealed one **without** fsyncing, dropping
    /// the open file handle.
    ///
    /// Unlike [`seal`](Self::seal), this is infallible: it performs no I/O. The
    /// caller MUST have already durably synced the segment's data (e.g. via
    /// [`sync_data`](Self::sync_data)) before calling this, because a sealed
    /// segment becomes non-last in the live ordering and a torn tail on a non-last
    /// segment is fatal during recovery. Doing the (fallible) fsync first and the
    /// (infallible) state transition second guarantees no fsync can fail *after*
    /// the segment has been swapped out — which would otherwise orphan a non-last
    /// segment that was dropped without being unlinked.
    ///
    /// `first_seq` / `max_seq` / `max_seq_explicit` are preserved unchanged.
    #[must_use]
    pub fn into_sealed(self) -> Self {
        let Segment {
            path,
            state: _,
            first_seq,
            max_seq,
            max_seq_explicit,
        } = self;
        // Dropping `state` (the Active file handle, if any) closes the descriptor.
        Self {
            path,
            state: SegmentState::Sealed,
            first_seq,
            max_seq,
            max_seq_explicit,
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_format_filename() {
        // New rotation format: partition + first_seq components.
        assert_eq!(
            parse_segment_filename("partition-005-00000000000000000042.log"),
            Some((5, 42))
        );
    }

    #[test]
    fn parse_legacy_single_file_filename() {
        // Legacy pre-rotation single file (`partition-NNN.log`, no first_seq) is
        // auto-migrated to first_seq 0 so its frames are recovered, not skipped.
        assert_eq!(parse_segment_filename("partition-007.log"), Some((7, 0)));
    }

    #[test]
    fn parse_rejects_non_segment_names() {
        assert_eq!(parse_segment_filename("partition-007.applied"), None);
        assert_eq!(parse_segment_filename("partition-abc.log"), None);
        assert_eq!(parse_segment_filename("unrelated.log"), None);
    }

    #[test]
    fn format_then_parse_round_trips() {
        let name = format_segment_filename(5, 42);
        assert_eq!(parse_segment_filename(&name), Some((5, 42)));
    }
}
