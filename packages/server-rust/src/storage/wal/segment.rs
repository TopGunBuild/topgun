//! WAL segment files: their headers, names and summaries, and a reader for
//! tests and reports.
//!
//! The log is a sequence of **stream segment** files
//! (`stream-{stream_seq:016}.log`), each starting with a
//! [`StreamSegmentHeader`] and holding the v3 frames of every partition, each
//! frame carrying its partition and sequence. The log writer (see
//! `log_writer`) appends to the single active one; collection unlinks a
//! sealed one once every partition's frames in it are at or below that
//! partition's applied watermark.
//!
//! Earlier versions wrote one log per partition: **per-partition segment**
//! files (`partition-{id:03}-{first_seq:020}.log`) whose name carries the
//! partition and the lowest sequence. A v2 one starts with a
//! [`SegmentHeader`] that repeats both and carries a random salt
//! (`TG-ENC-003`); a v1 one has no header and is a plain stream of v1 frames.
//! They are read and collected, never written.

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
        HeaderFields {
            layout: self.layout,
            payload_schema: self.payload_schema,
            cipher: self.cipher,
            key_id: self.key_id,
            salt: self.salt,
        }
        .encode(SEGMENT_HEADER_VERSION, &mut out);
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
        let fields = HeaderFields::parse(h, SEGMENT_HEADER_VERSION, SEGMENT_HEADER_BODY_LEN)?;
        Ok(Self {
            layout: fields.layout,
            payload_schema: fields.payload_schema,
            cipher: fields.cipher,
            key_id: fields.key_id,
            salt: fields.salt,
            partition: u32::from_be_bytes(h[45..49].try_into().expect("4 bytes")),
            first_seq: u64::from_be_bytes(h[49..57].try_into().expect("8 bytes")),
        })
    }
}

/// The fields every segment header version shares, at the same offsets:
/// `MAGIC u32 ‖ header_version u8 ‖ layout u8 ‖ payload_schema u16 BE ‖
/// cipher u8 ‖ key_id u32 BE ‖ salt [32]`, bytes 0..45.
struct HeaderFields {
    layout: SegmentLayout,
    payload_schema: PayloadSchema,
    cipher: SegmentCipher,
    key_id: u32,
    salt: [u8; 32],
}

impl HeaderFields {
    /// Writes the shared fields of a header of `version` into `out[..45]`.
    fn encode(&self, version: u8, out: &mut [u8]) {
        out[0..4].copy_from_slice(&SEGMENT_MAGIC.to_be_bytes());
        out[4] = version;
        out[5] = self.layout as u8;
        out[6..8].copy_from_slice(&(self.payload_schema as u16).to_be_bytes());
        out[8] = self.cipher as u8;
        out[9..13].copy_from_slice(&self.key_id.to_be_bytes());
        out[13..45].copy_from_slice(&self.salt);
    }

    /// Checks the CRC32C of a header of `body_len` body bytes and its
    /// version, and parses the shared fields.
    fn parse(h: &[u8], version: u8, body_len: usize) -> Result<Self, HeaderError> {
        let stored = u32::from_be_bytes(h[body_len..body_len + 4].try_into().expect("4 bytes"));
        let computed = crc32c::crc32c(&h[..body_len]);
        if stored != computed {
            return Err(HeaderError::BadCrc { stored, computed });
        }
        if h[4] != version {
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
        let key_id = u32::from_be_bytes(h[9..13].try_into().expect("4 bytes"));
        if (cipher == SegmentCipher::None) != (key_id == 0) {
            return Err(HeaderError::KeyIdMismatch { cipher, key_id });
        }
        Ok(Self {
            layout,
            payload_schema,
            cipher,
            key_id,
            salt: h[13..45].try_into().expect("32 bytes"),
        })
    }
}

// ---------------------------------------------------------------------------
// Stream segment header (v3)
// ---------------------------------------------------------------------------

/// Header version of a stream segment: one node-wide log whose frames carry
/// their partition and sequence.
pub const STREAM_HEADER_VERSION: u8 = 3;

/// Stream header bytes the CRC32C covers: magic through `stream_seq`.
pub const STREAM_HEADER_BODY_LEN: usize = 53;

/// Total bytes of a stream segment header: its body and a CRC32C u32 BE.
pub const STREAM_HEADER_LEN: usize = STREAM_HEADER_BODY_LEN + 4;

/// The header every stream segment starts with:
/// `MAGIC u32 ‖ header_version u8 (3) ‖ layout u8 ‖ payload_schema u16 BE ‖
/// cipher u8 ‖ key_id u32 BE ‖ salt [32] ‖ stream_seq u64 BE ‖ CRC32C u32 BE`,
/// 57 bytes. It names no partition: every frame carries its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSegmentHeader {
    pub layout: SegmentLayout,
    pub payload_schema: PayloadSchema,
    pub cipher: SegmentCipher,
    /// The data key the segment is sealed with; 0 iff `cipher` is `None`.
    pub key_id: u32,
    /// Random per created file, so no two files share an identity.
    pub salt: [u8; 32],
    /// Position of the segment in the stream; also its file name.
    pub stream_seq: u64,
}

impl StreamSegmentHeader {
    /// An unsealed header for stream segment `stream_seq`, with a fresh random
    /// salt.
    #[must_use]
    pub fn new(stream_seq: u64) -> Self {
        Self {
            layout: SegmentLayout::Fnv1aMod271,
            payload_schema: PayloadSchema::WalEntryV1,
            cipher: SegmentCipher::None,
            key_id: 0,
            salt: rand::random(),
            stream_seq,
        }
    }

    /// The header's 57 bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; STREAM_HEADER_LEN] {
        let mut out = [0u8; STREAM_HEADER_LEN];
        HeaderFields {
            layout: self.layout,
            payload_schema: self.payload_schema,
            cipher: self.cipher,
            key_id: self.key_id,
            salt: self.salt,
        }
        .encode(STREAM_HEADER_VERSION, &mut out);
        out[45..53].copy_from_slice(&self.stream_seq.to_be_bytes());
        let crc = crc32c::crc32c(&out[..STREAM_HEADER_BODY_LEN]);
        out[STREAM_HEADER_BODY_LEN..].copy_from_slice(&crc.to_be_bytes());
        out
    }

    /// Parses the stream header at the start of `bytes`, which must hold at
    /// least [`STREAM_HEADER_LEN`] bytes and start with [`SEGMENT_MAGIC`]; the
    /// caller classifies a shorter file.
    ///
    /// # Errors
    ///
    /// A CRC mismatch, a version other than [`STREAM_HEADER_VERSION`], or a
    /// field value this binary does not know.
    ///
    /// # Panics
    ///
    /// Never for input of at least [`STREAM_HEADER_LEN`] bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, HeaderError> {
        let h = &bytes[..STREAM_HEADER_LEN];
        let fields = HeaderFields::parse(h, STREAM_HEADER_VERSION, STREAM_HEADER_BODY_LEN)?;
        Ok(Self {
            layout: fields.layout,
            payload_schema: fields.payload_schema,
            cipher: fields.cipher,
            key_id: fields.key_id,
            salt: fields.salt,
            stream_seq: u64::from_be_bytes(h[45..53].try_into().expect("8 bytes")),
        })
    }
}

/// Where one frame of a stream segment sits, so a partition's frames are read
/// back without reading any other partition's. 32 bytes in memory per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLocation {
    pub sequence: u64,
    /// File offset of the frame's first byte.
    pub offset: u64,
    /// Position of the frame in its segment: the nonce of a sealed frame.
    pub index: u64,
    /// Bytes of the frame, header included.
    pub len: u32,
}

/// What one stream segment holds, read from its frame headers alone
/// (partition and sequence are in the clear), so collection judges a segment
/// and recovery finds a partition's frames without decrypting a payload: the
/// highest sequence of every partition with a frame there, and where each of
/// its frames sits. The index costs 32 bytes per frame (`FrameLocation`; each
/// partition's `Vec` is shrunk to fit once the open has read the segment) plus
/// one map entry and one `Vec` per partition per segment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SegmentSummary {
    pub max_seq: std::collections::BTreeMap<u32, u64>,
    /// Every frame of each partition, in file order.
    pub frames: std::collections::BTreeMap<u32, Vec<FrameLocation>>,
}

impl SegmentSummary {
    /// Counts the frame of `partition` at `location`.
    pub fn note(&mut self, partition: u32, location: FrameLocation) {
        let max = self.max_seq.entry(partition).or_insert(location.sequence);
        *max = (*max).max(location.sequence);
        self.frames.entry(partition).or_default().push(location);
    }
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

/// Filename prefix of a stream segment. A stream file keeps the `.log` suffix,
/// so tooling that sums `*.log` bytes counts it, while
/// [`parse_segment_filename`] never takes it for a partition segment.
pub const STREAM_FILE_PREFIX: &str = "stream-";

/// Zero-padding width of `stream_seq` in a stream segment's filename, so the
/// stream's files sort by name in stream order.
pub const STREAM_SEQ_PAD: usize = 16;

/// The filename of stream segment `stream_seq`: `stream-{stream_seq:016}.log`.
#[must_use]
pub fn format_stream_filename(stream_seq: u64) -> String {
    let pad = STREAM_SEQ_PAD;
    format!("{STREAM_FILE_PREFIX}{stream_seq:0pad$}{SEGMENT_FILE_SUFFIX}")
}

/// The `stream_seq` of a stream segment's filename, which carries it in
/// exactly [`STREAM_SEQ_PAD`] digits; `None` for any other name, so two names
/// can never denote one segment.
#[must_use]
pub fn parse_stream_filename(name: &str) -> Option<u64> {
    name.strip_prefix(STREAM_FILE_PREFIX)?
        .strip_suffix(SEGMENT_FILE_SUFFIX)
        .filter(|digits| {
            digits.len() == STREAM_SEQ_PAD && digits.bytes().all(|b| b.is_ascii_digit())
        })?
        .parse::<u64>()
        .ok()
}

// ---------------------------------------------------------------------------
// inspect — a reader for tests and reports
// ---------------------------------------------------------------------------

/// Reads the segment files of a WAL directory whatever their layout — v1 and
/// v2 per-partition segments and v3 stream segments — for tests, benches and
/// reports, so nothing outside the WAL module names a segment file or knows
/// its layout. Not a recovery path: it holds no keyring, so a sealed segment
/// is an error.
#[doc(hidden)]
pub mod inspect {
    use std::path::{Path, PathBuf};

    use super::{parse_segment_filename, parse_stream_filename};
    use crate::storage::wal::{format, WalEntry, WalWriter};

    /// One frame on disk.
    #[derive(Debug, Clone)]
    pub struct DiskFrame {
        /// The partition the frame belongs to: its file's for a per-partition
        /// segment, its own for a stream frame.
        pub partition: u32,
        pub sequence: u64,
        pub entry: WalEntry,
        /// Bytes the frame takes in its file, header included.
        pub encoded_len: usize,
    }

    /// A segment file and where it sits in the order it is read in.
    enum Listed {
        /// A per-partition segment of the partition.
        Partition(u32),
        /// A stream segment: its `stream_seq`.
        Stream(u64),
    }

    /// Every segment file of `wal_dir`: per-partition segments by partition
    /// and first sequence, then stream segments in stream order.
    fn listed(wal_dir: &Path) -> Vec<(Listed, PathBuf)> {
        let Ok(read_dir) = std::fs::read_dir(wal_dir) else {
            return Vec::new();
        };
        let mut partition = Vec::new();
        let mut stream = Vec::new();
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some((p, first_seq)) = parse_segment_filename(&name) {
                partition.push(((p, first_seq), entry.path()));
            } else if let Some(stream_seq) = parse_stream_filename(&name) {
                stream.push((stream_seq, entry.path()));
            }
        }
        partition.sort_by_key(|(at, _)| *at);
        stream.sort_by_key(|(at, _)| *at);
        partition
            .into_iter()
            .map(|((p, _), path)| (Listed::Partition(p), path))
            .chain(
                stream
                    .into_iter()
                    .map(|(stream_seq, path)| (Listed::Stream(stream_seq), path)),
            )
            .collect()
    }

    /// Every segment file of `wal_dir`, per-partition segments first; empty
    /// when the directory cannot be read.
    #[must_use]
    pub fn segment_paths(wal_dir: &Path) -> Vec<PathBuf> {
        listed(wal_dir).into_iter().map(|(_, path)| path).collect()
    }

    /// The summed size of every segment file of `wal_dir`; a file that
    /// vanishes while it is measured counts 0.
    #[must_use]
    pub fn bytes_on_disk(wal_dir: &Path) -> u64 {
        segment_paths(wal_dir)
            .iter()
            .map(|path| std::fs::metadata(path).map_or(0, |m| m.len()))
            .sum()
    }

    /// Every frame of every plaintext segment file of `wal_dir`, in the order
    /// of [`segment_paths`] and, within a file, in file order. A torn tail is
    /// tolerated where recovery tolerates it — on the last segment of a
    /// partition and on the newest stream segment — and is an error anywhere
    /// else, as is any other corruption or a sealed segment.
    ///
    /// # Errors
    ///
    /// A file that cannot be read, or one recovery would refuse.
    pub fn frames_on_disk(wal_dir: &Path) -> anyhow::Result<Vec<DiskFrame>> {
        let files = listed(wal_dir);
        let newest_stream = files.iter().rev().find_map(|(listed, _)| match listed {
            Listed::Stream(stream_seq) => Some(*stream_seq),
            Listed::Partition(_) => None,
        });
        let mut frames = Vec::new();
        for (idx, (listed, path)) in files.iter().enumerate() {
            let data = std::fs::read(path)
                .map_err(|e| anyhow::anyhow!("Cannot read WAL segment {}: {e}", path.display()))?;
            match listed {
                Listed::Partition(partition) => {
                    let last_of_partition = !matches!(
                        files.get(idx + 1),
                        Some((Listed::Partition(next), _)) if next == partition
                    );
                    let decoded = format::decode_segment(&data);
                    let spans = decoded.spans.clone();
                    let (entries, _) =
                        WalWriter::entries_or_refuse(path, decoded, last_of_partition)?;
                    frames.extend(spans.iter().zip(entries).map(|(span, entry)| DiskFrame {
                        partition: *partition,
                        sequence: entry.sequence,
                        entry,
                        encoded_len: span.len,
                    }));
                }
                Listed::Stream(stream_seq) => {
                    let (_, spans, entries) = WalWriter::read_stream_segment(
                        path,
                        &data,
                        Some(*stream_seq) == newest_stream,
                        None,
                        true,
                    )?;
                    for (span, entry) in spans.iter().zip(entries) {
                        let (partition, sequence) = span.address.ok_or_else(|| {
                            anyhow::anyhow!("a frame of {} has no address", path.display())
                        })?;
                        frames.push(DiskFrame {
                            partition,
                            sequence,
                            entry,
                            encoded_len: span.len,
                        });
                    }
                }
            }
        }
        Ok(frames)
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

    #[test]
    fn a_stream_name_is_never_a_partition_segment() {
        let name = format_stream_filename(7);
        assert_eq!(name, "stream-0000000000000007.log");
        assert_eq!(parse_stream_filename(&name), Some(7));
        assert_eq!(parse_segment_filename(&name), None);
        assert_eq!(
            parse_stream_filename("partition-005-00000000000000000042.log"),
            None
        );
        assert_eq!(parse_stream_filename("stream-+7.log"), None);
        assert_eq!(parse_stream_filename("stream-.log"), None);
        assert_eq!(parse_stream_filename("stream-1.log"), None);
        assert_eq!(parse_stream_filename("stream-00000000000000001.log"), None);
    }

    fn entry(map: &str, sequence: u64) -> crate::storage::wal::WalEntry {
        crate::storage::wal::WalEntry {
            map: map.to_string(),
            key: format!("k{sequence}"),
            op: crate::storage::wal::WalOp::Remove,
            timestamp: None,
            sequence,
        }
    }

    /// A plaintext stream segment holding `frames` as `(partition, sequence)`.
    pub(crate) fn stream_segment(stream_seq: u64, frames: &[(u32, u64)]) -> Vec<u8> {
        let mut data = StreamSegmentHeader::new(stream_seq).encode().to_vec();
        for &(partition, sequence) in frames {
            let payload =
                crate::storage::wal::format::encode_v2_payload(&entry("m", sequence), false)
                    .unwrap();
            data.extend(
                crate::storage::wal::format::frame_v3(partition, sequence, &payload, None).unwrap(),
            );
        }
        data
    }

    #[test]
    fn r_b_stream_header_round_trips_and_refuses_unknown_fields_by_name() {
        let mut header = StreamSegmentHeader::new(42);
        header.cipher = SegmentCipher::XChaCha20Poly1305;
        header.key_id = 3;
        let bytes = header.encode();
        assert_eq!(bytes.len(), STREAM_HEADER_LEN);
        assert_eq!(StreamSegmentHeader::parse(&bytes), Ok(header));

        // Each refused field is named, with its value, after a valid CRC.
        let refused = |at: usize, value: u8| {
            let mut changed = bytes;
            changed[at] = value;
            let crc = crc32c::crc32c(&changed[..STREAM_HEADER_BODY_LEN]);
            changed[STREAM_HEADER_BODY_LEN..].copy_from_slice(&crc.to_be_bytes());
            StreamSegmentHeader::parse(&changed)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(refused(4, 2), "unknown header version 2");
        assert_eq!(refused(5, 9), "unknown partition layout 9");
        assert_eq!(refused(7, 9), "unknown payload schema 9");
        assert_eq!(refused(8, 9), "unknown cipher 9");
        // A per-partition (v2) header is not a stream header, nor the reverse.
        let v2 = SegmentHeader::new_unsealed(1, 1).encode();
        let mut as_stream = v2[..STREAM_HEADER_LEN].to_vec();
        let crc = crc32c::crc32c(&as_stream[..STREAM_HEADER_BODY_LEN]);
        as_stream[STREAM_HEADER_BODY_LEN..].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(
            StreamSegmentHeader::parse(&as_stream),
            Err(HeaderError::UnknownVersion(2))
        );
        let mut as_v2 = [0u8; SEGMENT_HEADER_LEN];
        as_v2[..STREAM_HEADER_LEN].copy_from_slice(&bytes);
        let crc = crc32c::crc32c(&as_v2[..SEGMENT_HEADER_BODY_LEN]);
        as_v2[SEGMENT_HEADER_BODY_LEN..].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(
            SegmentHeader::parse(&as_v2),
            Err(HeaderError::UnknownVersion(STREAM_HEADER_VERSION))
        );
    }

    #[test]
    fn r_b_stream_frames_round_trip_with_their_partition_and_sequence() {
        use crate::storage::wal::format::{decode_stream_segment, FrameDecodeResult};
        let data = stream_segment(9, &[(4, 7), (270, 2), (4, 8)]);
        let decoded = decode_stream_segment(&data, None, true);
        assert_eq!(decoded.header.map(|h| h.stream_seq), Some(9));
        let FrameDecodeResult::Complete(entries) = decoded.result else {
            panic!("{:?}", decoded.result);
        };
        assert_eq!(
            entries.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            vec![7, 2, 8]
        );
        assert_eq!(
            decoded.spans.iter().map(|s| s.address).collect::<Vec<_>>(),
            vec![Some((4, 7)), Some((270, 2)), Some((4, 8))]
        );
        assert_eq!(decoded.intact_len, data.len());
        // The index the summary keeps costs what its doc states per frame.
        assert_eq!(std::mem::size_of::<FrameLocation>(), 32);
        // The envelopes alone give the same addresses without reading a payload.
        let headers = decode_stream_segment(&data, None, false);
        assert_eq!(headers.spans, decoded.spans);

        // The CRC covers the partition: a changed one is corruption, not a
        // frame of another partition.
        let mut changed = data.clone();
        changed[STREAM_HEADER_LEN + 13] ^= 0x01;
        assert!(matches!(
            decode_stream_segment(&changed, None, false).result,
            FrameDecodeResult::Corruption { offset, .. } if offset == STREAM_HEADER_LEN
        ));
    }

    #[test]
    fn r_b_inspect_reads_every_layout() {
        use crate::storage::wal::format;
        let dir = tempfile::tempdir().unwrap();
        let wal_dir = dir.path();
        // v1: a headerless legacy file of partition 7.
        let v1_path = wal_dir.join("partition-007.log");
        let mut v1 = format::encode(&entry("a", 1)).unwrap();
        v1.extend(format::encode(&entry("a", 2)).unwrap());
        std::fs::write(&v1_path, &v1).unwrap();
        // v2: a headed segment of partition 7 from sequence 3.
        let v2_path = wal_dir.join(format_segment_filename(7, 3));
        let mut v2 = SegmentHeader::new_unsealed(7, 3).encode().to_vec();
        v2.extend(format::encode_v2(&entry("a", 3)).unwrap());
        std::fs::write(&v2_path, &v2).unwrap();
        // v3: a stream segment with frames of two partitions.
        let v3_path = wal_dir.join(format_stream_filename(1));
        let v3 = stream_segment(1, &[(9, 5), (7, 4)]);
        std::fs::write(&v3_path, &v3).unwrap();
        std::fs::write(wal_dir.join("wal-collected"), b"not a segment").unwrap();

        assert_eq!(
            inspect::segment_paths(wal_dir),
            vec![v1_path.clone(), v2_path.clone(), v3_path.clone()]
        );
        assert_eq!(
            inspect::bytes_on_disk(wal_dir),
            (v1.len() + v2.len() + v3.len()) as u64
        );
        let frames = inspect::frames_on_disk(wal_dir).unwrap();
        assert_eq!(
            frames
                .iter()
                .map(|f| (f.partition, f.sequence, f.entry.sequence))
                .collect::<Vec<_>>(),
            vec![(7, 1, 1), (7, 2, 2), (7, 3, 3), (9, 5, 5), (7, 4, 4)]
        );
        assert_eq!(
            frames.iter().map(|f| f.encoded_len).sum::<usize>(),
            v1.len() + (v2.len() - SEGMENT_HEADER_LEN) + (v3.len() - STREAM_HEADER_LEN)
        );

        // A torn tail on the newest stream segment is tolerated.
        std::fs::write(&v3_path, &v3[..v3.len() - 3]).unwrap();
        let frames = inspect::frames_on_disk(wal_dir).unwrap();
        assert_eq!(frames.last().map(|f| f.sequence), Some(5));

        // On an older segment it is an error: the v1 file is followed by the
        // v2 segment of its partition, and an older stream segment by a newer.
        std::fs::write(&v1_path, &v1[..v1.len() - 3]).unwrap();
        assert!(inspect::frames_on_disk(wal_dir).is_err());
        std::fs::write(&v1_path, &v1).unwrap();
        std::fs::write(
            wal_dir.join(format_stream_filename(2)),
            stream_segment(2, &[(9, 6)]),
        )
        .unwrap();
        let err = inspect::frames_on_disk(wal_dir).unwrap_err().to_string();
        assert!(err.contains(&v3_path.display().to_string()), "{err}");
    }
}
