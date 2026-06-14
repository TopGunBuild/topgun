//! On-disk frame codec for the Write-Ahead Log.
//!
//! Each frame has the structure:
//!
//! ```text
//! [ MAGIC (4 bytes) | VERSION (1 byte) | LENGTH (4 bytes) | CRC32C (4 bytes) | PAYLOAD (LENGTH bytes) ]
//! ```
//!
//! - **MAGIC** (`0x54_47_57_4C` = ASCII "TGWL"): lets recovery distinguish
//!   a WAL frame from arbitrary bytes and from frames written by an incompatible
//!   format. A magic mismatch is a distinct refusal, NOT a CRC mismatch.
//! - **VERSION** (`1u8`): format-version byte. An unrecognised version produces
//!   its own distinct error (`UnknownVersion`) so a future format change is
//!   detectable rather than mis-reported as corruption.
//! - **LENGTH** (big-endian `u32`): byte length of the `MsgPack` payload.
//! - **CRC32C** (big-endian `u32`): Castagnoli CRC-32 of the payload bytes.
//!   A mismatch that occurs before the last frame is `Corruption`; a mismatch on
//!   the last frame that is also truncated is reported as `TruncatedTail`.
//! - **PAYLOAD**: `rmp_serde::to_vec_named()` `MsgPack` encoding of `WalEntry`.
//!
//! CRC32C (Castagnoli) is used rather than CRC-32 IEEE because it has hardware
//! acceleration on modern CPUs and is the standard choice for WAL frames
//! (Postgres WAL, `RocksDB` log). The `crc32c` crate provides the genuine
//! Castagnoli polynomial.

use std::io::{self, Cursor, Read};

use crate::storage::wal::WalEntry;

// ---------------------------------------------------------------------------
// Frame constants
// ---------------------------------------------------------------------------

/// Frame magic: ASCII "TGWL" (`TopGun` Write-ahead Log).
///
/// Written at the start of every frame so a reader can immediately distinguish
/// a valid frame from arbitrary bytes or a frame from an incompatible format.
pub const FRAME_MAGIC: u32 = 0x54_47_57_4C;

/// Current on-disk format version.
///
/// Bumped whenever the frame layout changes in a backward-incompatible way so
/// recovery can refuse to read frames written by a newer binary rather than
/// silently mis-interpreting them as corruption.
pub const FRAME_VERSION: u8 = 1;

/// Total header bytes: 4 (magic) + 1 (version) + 4 (length) + 4 (crc32c).
pub const FRAME_HEADER_LEN: usize = 13;

/// Upper bound on a single frame's declared payload length.
///
/// The length field is an untrusted on-disk `u32` — a torn write that corrupts
/// only the length while leaving valid magic+version (a plausible partial-sector
/// write) could declare up to `u32::MAX` (~4 GiB). `decode_all` would then
/// `vec![0u8; payload_len]` **before** verifying the CRC, so a single corrupt
/// length turns recovery into a multi-gigabyte allocation (OOM / crash-loop on
/// startup). We reject any frame whose declared length exceeds this ceiling
/// rather than allocate for it.
///
/// 64 MiB is far above any legitimate `WalEntry` (one CRDT record: map + key +
/// value) yet small enough that a rejection allocates nothing. This mirrors the
/// per-record size caps in Postgres WAL / `RocksDB` log / tikv `raft-engine`,
/// none of which allocate for an unbounded on-disk length.
pub const MAX_FRAME_PAYLOAD_LEN: usize = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// FrameDecodeResult
// ---------------------------------------------------------------------------

/// The typed outcome of decoding a stream of WAL frames.
///
/// Recovery code MUST pattern-match on all variants — panicking on an
/// unexpected variant would turn a detectable format error into a crash.
#[derive(Debug)]
pub enum FrameDecodeResult {
    /// The stream ended cleanly on a frame boundary (empty or fully-consumed).
    CleanEof,
    /// The final frame in the stream is incomplete (truncated).
    ///
    /// This is a recoverable condition: the writer crashed mid-write, so the
    /// prefix of complete frames is valid. Recovery should replay all complete
    /// frames and discard the partial tail.
    TruncatedTail {
        /// Fully-decoded entries from all complete frames before the truncation.
        complete: Vec<WalEntry>,
    },
    /// A frame whose magic marker does not match `FRAME_MAGIC`.
    ///
    /// Distinct from `Corruption` so a consumer can tell the difference between
    /// a CRC error (possibly random bit-flip) and a completely wrong file type
    /// or an accidentally-overwritten WAL segment.
    BadMagic {
        /// Offset in bytes where the bad magic was found.
        offset: usize,
    },
    /// A frame with a recognised magic and version, but an unrecognised version
    /// byte.
    ///
    /// Distinct from both `BadMagic` and `Corruption` so a future format change
    /// is detectable rather than silently mis-classified.
    UnknownVersion {
        /// The version byte that was read.
        found: u8,
        /// Offset where the frame header began.
        offset: usize,
    },
    /// A frame with correct magic, version, and length, but whose CRC32C does not
    /// match the payload.
    ///
    /// This indicates mid-file corruption (bit-flip, torn write on a non-final
    /// frame). Recovery should refuse to replay anything past this point.
    Corruption {
        /// Offset of the corrupted frame.
        offset: usize,
        /// The CRC stored in the frame header.
        stored_crc: u32,
        /// The CRC computed from the payload bytes.
        computed_crc: u32,
    },
    /// All frames decoded successfully.
    Complete(Vec<WalEntry>),
}

// ---------------------------------------------------------------------------
// encode
// ---------------------------------------------------------------------------

/// Encodes a `WalEntry` into a length-prefixed, CRC32C-checksummed WAL frame.
///
/// Uses `rmp_serde::to_vec_named()` for `MsgPack` encoding with named fields to
/// stay wire-compatible with the rest of the `TopGun` `MsgPack` protocol.
///
/// # Errors
///
/// Returns an error if `MsgPack` serialization fails (e.g., the entry contains
/// a value that cannot be represented in `MsgPack`).
pub fn encode(entry: &WalEntry) -> anyhow::Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(entry)
        .map_err(|e| anyhow::anyhow!("WAL encode: MsgPack serialization failed: {e}"))?;

    let length = u32::try_from(payload.len())
        .map_err(|_| anyhow::anyhow!("WAL encode: payload too large ({} bytes)", payload.len()))?;

    let crc = crc32c::crc32c(&payload);

    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    frame.extend_from_slice(&FRAME_MAGIC.to_be_bytes());
    frame.push(FRAME_VERSION);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&crc.to_be_bytes());
    frame.extend_from_slice(&payload);

    Ok(frame)
}

// ---------------------------------------------------------------------------
// decode_all
// ---------------------------------------------------------------------------

/// Decodes all WAL frames from `data`, returning a typed `FrameDecodeResult`.
///
/// The caller should inspect the variant to decide how to proceed:
/// - `Complete` → all frames healthy, replay the entries.
/// - `CleanEof` → file is empty or ends on a frame boundary — nothing to replay.
/// - `TruncatedTail` → replay `complete`, discard the partial frame.
/// - `BadMagic` / `UnknownVersion` / `Corruption` → refuse; signal operator.
///
/// This function NEVER panics on malformed input.
#[must_use]
pub fn decode_all(data: &[u8]) -> FrameDecodeResult {
    if data.is_empty() {
        return FrameDecodeResult::CleanEof;
    }

    let mut cursor = Cursor::new(data);
    let mut entries = Vec::new();
    let total_len = data.len();

    loop {
        let offset = usize::try_from(cursor.position()).unwrap_or(usize::MAX);

        // --- Check for clean EOF on a frame boundary ---
        if offset == total_len {
            return FrameDecodeResult::Complete(entries);
        }

        // --- Read magic (4 bytes) ---
        let mut magic_buf = [0u8; 4];
        match cursor.read_exact(&mut magic_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                // Could not read a full magic — tail is truncated.
                return FrameDecodeResult::TruncatedTail { complete: entries };
            }
            Err(_) => return FrameDecodeResult::TruncatedTail { complete: entries },
        }

        let magic = u32::from_be_bytes(magic_buf);
        if magic != FRAME_MAGIC {
            return FrameDecodeResult::BadMagic { offset };
        }

        // --- Read version (1 byte) ---
        let mut version_buf = [0u8; 1];
        match cursor.read_exact(&mut version_buf) {
            Ok(()) => {}
            Err(_) => return FrameDecodeResult::TruncatedTail { complete: entries },
        }
        let version = version_buf[0];
        if version != FRAME_VERSION {
            return FrameDecodeResult::UnknownVersion {
                found: version,
                offset,
            };
        }

        // --- Read length (4 bytes, big-endian u32) ---
        let mut length_buf = [0u8; 4];
        match cursor.read_exact(&mut length_buf) {
            Ok(()) => {}
            Err(_) => return FrameDecodeResult::TruncatedTail { complete: entries },
        }
        let payload_len = u32::from_be_bytes(length_buf) as usize;

        // --- Read CRC32C (4 bytes) ---
        let mut crc_buf = [0u8; 4];
        match cursor.read_exact(&mut crc_buf) {
            Ok(()) => {}
            Err(_) => return FrameDecodeResult::TruncatedTail { complete: entries },
        }
        let stored_crc = u32::from_be_bytes(crc_buf);

        // --- Bound the declared payload length BEFORE allocating ---
        // The length field is untrusted on-disk data; a corrupt/torn length must
        // never drive the allocation below. Two distinct rejections, ordered so a
        // crash-mid-append (the common case) stays recoverable rather than fatal:
        //
        // 1. `payload_len > remaining` — the frame extends past the end of the
        //    data, so it cannot be a complete frame. This is the torn-tail case
        //    (writer crashed mid-append); replay the intact prefix. This also
        //    catches the ~4 GiB bloated-length-on-a-small-file DoS, because a huge
        //    declared length always exceeds the bytes actually present, so we
        //    return here without allocating `payload_len`.
        // 2. `payload_len > MAX_FRAME_PAYLOAD_LEN` — enough bytes are present for
        //    the declared length, but it exceeds any legitimate frame. Refuse
        //    fatally (Corruption) rather than allocate: with valid frames possibly
        //    following, a silent truncation could drop acked writes.
        let remaining =
            total_len.saturating_sub(usize::try_from(cursor.position()).unwrap_or(usize::MAX));
        if payload_len > remaining {
            return FrameDecodeResult::TruncatedTail { complete: entries };
        }
        if payload_len > MAX_FRAME_PAYLOAD_LEN {
            return FrameDecodeResult::Corruption {
                offset,
                stored_crc,
                // The frame is rejected on the length bound before its CRC is
                // computed; 0 is a sentinel "not computed", stored_crc is real.
                computed_crc: 0,
            };
        }

        // --- Read payload ---
        let mut payload = vec![0u8; payload_len];
        match cursor.read_exact(&mut payload) {
            Ok(()) => {}
            Err(_) => {
                // Partial payload at end of stream — truncated tail.
                return FrameDecodeResult::TruncatedTail { complete: entries };
            }
        }

        // --- Verify CRC32C ---
        let computed_crc = crc32c::crc32c(&payload);
        if computed_crc != stored_crc {
            // Determine whether this is the final frame or a mid-file corruption.
            let next_offset = usize::try_from(cursor.position()).unwrap_or(usize::MAX);
            if next_offset >= total_len {
                // CRC mismatch on the last frame — treat as truncated tail since
                // the writer may have crashed mid-checksum.
                return FrameDecodeResult::TruncatedTail { complete: entries };
            }
            // CRC mismatch before the end of the stream — mid-file corruption.
            return FrameDecodeResult::Corruption {
                offset,
                stored_crc,
                computed_crc,
            };
        }

        // --- Deserialize payload ---
        if let Ok(entry) = rmp_serde::from_slice::<WalEntry>(&payload) {
            entries.push(entry);
        } else {
            // Deserialisation failure on a frame that passed CRC is still treated
            // as corruption — the bytes are internally consistent but unreadable.
            let next_offset = usize::try_from(cursor.position()).unwrap_or(usize::MAX);
            if next_offset >= total_len {
                return FrameDecodeResult::TruncatedTail { complete: entries };
            }
            return FrameDecodeResult::Corruption {
                offset,
                stored_crc,
                computed_crc,
            };
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::wal::{WalEntry, WalOp};
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    fn make_store_entry(seq: u64) -> WalEntry {
        WalEntry {
            map: "test_map".to_string(),
            key: format!("key_{seq}"),
            op: WalOp::Store {
                value: Value::String("hello".to_string()),
                expiration_time: Some(1_700_000_000_000),
            },
            timestamp: Some(Timestamp {
                millis: 1_700_000_000_000,
                counter: 0,
                node_id: "node1".to_string(),
            }),
            sequence: seq,
        }
    }

    fn make_remove_entry(seq: u64) -> WalEntry {
        WalEntry {
            map: "test_map".to_string(),
            key: format!("key_{seq}"),
            op: WalOp::Remove,
            timestamp: None,
            sequence: seq,
        }
    }

    // -----------------------------------------------------------------------
    // AC1: Lossless round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn round_trip_store_entry() {
        let entry = make_store_entry(42);
        let encoded = encode(&entry).expect("encode should succeed");
        let result = decode_all(&encoded);
        match result {
            FrameDecodeResult::Complete(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0], entry);
            }
            other => panic!("Expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_remove_entry() {
        let entry = make_remove_entry(7);
        let encoded = encode(&entry).expect("encode should succeed");
        let result = decode_all(&encoded);
        match result {
            FrameDecodeResult::Complete(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0], entry);
            }
            other => panic!("Expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_multiple_frames() {
        let entries: Vec<WalEntry> = (0..5)
            .map(|i| {
                if i % 2 == 0 {
                    make_store_entry(i)
                } else {
                    make_remove_entry(i)
                }
            })
            .collect();

        let mut data = Vec::new();
        for e in &entries {
            data.extend_from_slice(&encode(e).unwrap());
        }

        match decode_all(&data) {
            FrameDecodeResult::Complete(decoded) => {
                assert_eq!(decoded, entries);
            }
            other => panic!("Expected Complete, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // AC1: Clean EOF on empty input
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input_is_clean_eof() {
        match decode_all(&[]) {
            FrameDecodeResult::CleanEof => {}
            other => panic!("Expected CleanEof, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // AC2: Truncated tail — last frame incomplete
    // -----------------------------------------------------------------------

    #[test]
    fn truncated_tail_is_recoverable() {
        // Encode two complete frames then truncate the second one mid-payload.
        let entry1 = make_store_entry(1);
        let entry2 = make_store_entry(2);

        let mut data = encode(&entry1).unwrap();
        let frame2 = encode(&entry2).unwrap();
        // Append only a partial second frame (half its bytes).
        let partial_len = frame2.len() / 2;
        data.extend_from_slice(&frame2[..partial_len]);

        match decode_all(&data) {
            FrameDecodeResult::TruncatedTail { complete } => {
                // The first complete frame should be returned for replay.
                assert_eq!(complete.len(), 1);
                assert_eq!(complete[0], entry1);
            }
            other => panic!("Expected TruncatedTail, got {other:?}"),
        }
    }

    #[test]
    fn truncated_tail_mid_header() {
        // Truncate right in the middle of the magic bytes.
        let entry = make_store_entry(1);
        let data = encode(&entry).unwrap();
        // Feed only the magic bytes of a second frame (not even a full header).
        let mut truncated = data.clone();
        truncated.extend_from_slice(&FRAME_MAGIC.to_be_bytes()[..2]);

        match decode_all(&truncated) {
            FrameDecodeResult::TruncatedTail { complete } => {
                assert_eq!(complete.len(), 1);
            }
            other => panic!("Expected TruncatedTail, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // AC2: Mid-file CRC mismatch — distinct from truncation
    // -----------------------------------------------------------------------

    #[test]
    fn mid_file_crc_mismatch_is_corruption() {
        // Encode two valid frames, then flip a byte in the first frame's payload.
        let entry1 = make_store_entry(1);
        let entry2 = make_store_entry(2);

        let mut frame1 = encode(&entry1).unwrap();
        let frame2 = encode(&entry2).unwrap();

        // Corrupt a byte in frame1's payload (offset = FRAME_HEADER_LEN).
        let payload_start = FRAME_HEADER_LEN;
        frame1[payload_start] ^= 0xFF;

        let mut data = frame1;
        data.extend_from_slice(&frame2);

        match decode_all(&data) {
            FrameDecodeResult::Corruption { offset, .. } => {
                // Corruption must be reported at frame 0 (offset 0), not confused
                // with TruncatedTail.
                assert_eq!(offset, 0);
            }
            other => panic!("Expected Corruption, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // AC2: Bad magic — distinct classification
    // -----------------------------------------------------------------------

    #[test]
    fn bad_magic_is_distinct_from_corruption() {
        // Write a frame with a wrong magic number.
        let entry = make_store_entry(1);
        let mut frame = encode(&entry).unwrap();
        // Overwrite the magic with something else.
        frame[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());

        match decode_all(&frame) {
            FrameDecodeResult::BadMagic { offset } => {
                assert_eq!(offset, 0);
            }
            other => panic!("Expected BadMagic, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // AC2: Unknown version — distinct classification
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Bounded payload allocation — a corrupt/bloated length field must never
    // drive a multi-gigabyte allocation during recovery (DoS / OOM hardening).
    // -----------------------------------------------------------------------

    /// Build a raw frame header (valid magic + version) with an arbitrary
    /// declared `length`, followed by `payload` bytes verbatim. The CRC field is
    /// filled with `0` because these tests exercise the length bound, which is
    /// checked *before* the CRC is computed.
    fn raw_frame_with_declared_len(length: u32, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
        frame.extend_from_slice(&FRAME_MAGIC.to_be_bytes());
        frame.push(FRAME_VERSION);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(&0u32.to_be_bytes()); // CRC placeholder
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn bloated_length_is_truncated_tail_without_allocating() {
        // A frame header claims a ~4 GiB payload but only a handful of bytes
        // follow. The decoder must NOT allocate `vec![0u8; 4 GiB]`; because the
        // declared length exceeds the bytes actually present, it is classified as
        // a truncated tail (crash-mid-append) and the intact prefix is returned.
        // Pre-fix this test would attempt a ~4 GiB allocation and OOM/abort.
        let data = raw_frame_with_declared_len(u32::MAX, &[1, 2, 3, 4]);

        match decode_all(&data) {
            FrameDecodeResult::TruncatedTail { complete } => {
                assert!(
                    complete.is_empty(),
                    "no complete frame precedes the bad one"
                );
            }
            other => panic!("Expected TruncatedTail (no allocation), got {other:?}"),
        }
    }

    #[test]
    fn bloated_length_after_valid_frame_keeps_prefix() {
        // A valid frame, then a frame declaring a ~4 GiB payload with no bytes
        // behind it. The valid prefix must survive; the bloated frame must be
        // treated as a truncated tail, never allocated.
        let entry = make_store_entry(1);
        let mut data = encode(&entry).unwrap();
        data.extend_from_slice(&raw_frame_with_declared_len(u32::MAX, &[]));

        match decode_all(&data) {
            FrameDecodeResult::TruncatedTail { complete } => {
                assert_eq!(complete.len(), 1);
                assert_eq!(complete[0], entry);
            }
            other => panic!("Expected TruncatedTail with intact prefix, got {other:?}"),
        }
    }

    #[test]
    fn oversized_but_present_payload_is_corruption() {
        // Enough bytes ARE present for the declared length, but it exceeds the
        // per-frame ceiling. This is genuine corruption (not a torn tail) and
        // must be refused, not allocated-and-replayed. Built as a single buffer
        // (header + zero payload in place) to avoid a transient double-copy of
        // the 64 MiB payload under parallel test memory pressure.
        let over = MAX_FRAME_PAYLOAD_LEN + 1;
        let mut data = vec![0u8; FRAME_HEADER_LEN + over];
        data[0..4].copy_from_slice(&FRAME_MAGIC.to_be_bytes());
        data[4] = FRAME_VERSION;
        data[5..9].copy_from_slice(
            &u32::try_from(over)
                .expect("over fits in u32 for this test")
                .to_be_bytes(),
        );
        // bytes 9..13 (CRC) and the payload stay zeroed; the length bound is
        // checked before the CRC, so the CRC value is irrelevant here.

        match decode_all(&data) {
            FrameDecodeResult::Corruption { offset, .. } => {
                assert_eq!(offset, 0, "the over-sized frame is the first frame");
            }
            other => panic!("Expected Corruption for an over-MAX frame, got {other:?}"),
        }
    }

    #[test]
    fn unknown_version_is_distinct_from_corruption_and_bad_magic() {
        let entry = make_store_entry(1);
        let mut frame = encode(&entry).unwrap();
        // Magic is correct (bytes 0..4), but version byte is unknown.
        frame[4] = 99; // not FRAME_VERSION (1)

        match decode_all(&frame) {
            FrameDecodeResult::UnknownVersion { found, offset } => {
                assert_eq!(found, 99);
                assert_eq!(offset, 0);
            }
            other => panic!("Expected UnknownVersion, got {other:?}"),
        }
    }
}
