//! Depth- and structure-bounded `MsgPack` decoding for untrusted inbound frames.
//!
//! Every inbound WebSocket frame (and the HTTP `/sync` body) is `MsgPack`, decoded
//! with `rmp_serde` — and on `/ws` the decode happens in Phase 1, BEFORE the
//! connection is authenticated. `rmp_serde` applies no recursion-depth limit,
//! and the message types are internally tagged (`#[serde(tag = "type")]`, which
//! buffers the whole frame into serde's recursive `Content` type) and embed
//! `rmpv::Value` — itself a recursive enum. A deeply-nested frame therefore
//! recurses one native stack frame per nesting level during deserialization, and
//! a stack overflow in safe Rust aborts the whole process (uncatchable: no
//! `catch_unwind`, no per-task isolation). One small unauthenticated frame would
//! kill every connection on the node.
//!
//! [`decode_depth_checked`] closes this by scanning the raw `MsgPack` bytes
//! iteratively — an explicit heap stack, so the scanner itself never recurses and
//! cannot overflow — and rejecting any frame whose container nesting exceeds
//! [`MAX_DECODE_DEPTH`] before the bytes ever reach `rmp_serde`. Paired with the
//! inbound frame-size cap on the WebSocket upgrade, this bounds the decoder's
//! input to well-formed, shallow frames.

use rmp::Marker;
use serde::de::DeserializeOwned;

/// Maximum container-nesting depth accepted for an inbound `MsgPack` frame.
///
/// Legitimate `TopGun` payloads (operation envelopes carrying `rmpv::Value`
/// records and `where` predicates) nest only a handful of levels deep; 128 is
/// far above any real document while still well below the thousands of levels it
/// takes to exhaust a worker thread's stack during `rmp_serde` recursion.
pub const MAX_DECODE_DEPTH: usize = 128;

/// Why an inbound frame was rejected before or during decode.
#[derive(Debug)]
pub enum DecodeError {
    /// Container nesting exceeded the allowed depth — rejected before `rmp_serde`
    /// ever ran, so the recursive decoder cannot overflow the stack.
    TooDeep,
    /// The byte stream is not well-formed `MsgPack` (truncated, or a reserved
    /// marker), detected by the pre-scan.
    Malformed,
    /// Structurally valid and within depth, but failed to deserialize into the
    /// target type.
    Decode(rmp_serde::decode::Error),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooDeep => write!(
                f,
                "frame rejected: MsgPack nesting exceeds {MAX_DECODE_DEPTH} levels"
            ),
            Self::Malformed => write!(f, "frame rejected: malformed MsgPack"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::TooDeep | Self::Malformed => None,
        }
    }
}

/// Reads `n` big-endian length bytes at `*pos`, advancing the cursor.
/// Bounds-checked: returns [`DecodeError::Malformed`] on truncation.
fn read_uint(data: &[u8], pos: &mut usize, n: usize) -> Result<u64, DecodeError> {
    let end = pos.checked_add(n).ok_or(DecodeError::Malformed)?;
    let slice = data.get(*pos..end).ok_or(DecodeError::Malformed)?;
    let mut v = 0u64;
    for &b in slice {
        v = (v << 8) | u64::from(b);
    }
    *pos = end;
    Ok(v)
}

/// Advances `*pos` past `n` payload bytes (str/bin/ext/scalar bodies) without
/// reading them. Bounds-checked: returns [`DecodeError::Malformed`] on truncation.
fn skip(data: &[u8], pos: &mut usize, n: u64) -> Result<(), DecodeError> {
    let n = usize::try_from(n).map_err(|_| DecodeError::Malformed)?;
    let end = pos.checked_add(n).ok_or(DecodeError::Malformed)?;
    if end > data.len() {
        return Err(DecodeError::Malformed);
    }
    *pos = end;
    Ok(())
}

/// Scans `data` as `MsgPack` iteratively, bounding container nesting to `max_depth`.
///
/// Returns `Err(TooDeep)` the moment nesting would exceed `max_depth`, and
/// `Err(Malformed)` on a truncated stream or reserved marker. The scan uses an
/// explicit heap stack and **never recurses**, so it cannot itself overflow on
/// adversarial input — that is the whole point: it runs before the recursive
/// `rmp_serde` decoder and rejects frames that would overflow it.
///
/// Trailing bytes after the first complete top-level value are ignored here;
/// `rmp_serde` rejects them downstream if it cares.
///
/// # Errors
///
/// Returns [`DecodeError::TooDeep`] if container nesting would exceed
/// `max_depth`, or [`DecodeError::Malformed`] on a truncated stream or a
/// reserved marker byte.
// One match arm per MsgPack marker keeps the scanner auditable; merging arms or
// splitting the function would obscure the byte-advance accounting that makes it
// safe, so the length and a few same-bodied arms are intentional.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
pub fn check_msgpack_depth(data: &[u8], max_depth: usize) -> Result<(), DecodeError> {
    let mut pos = 0usize;

    // `stack[i]` = number of child values still to read at nesting level `i`.
    // `pending` is the count at the current (deepest open) level. The scanner
    // expects exactly one top-level value.
    let mut stack: Vec<u64> = Vec::new();
    let mut pending: u64 = 1;

    loop {
        // Pop levels whose child count has been fully consumed.
        while pending == 0 {
            match stack.pop() {
                Some(parent_remaining) => pending = parent_remaining,
                None => return Ok(()), // all values consumed
            }
        }
        // Consume one value at the current level.
        pending -= 1;

        let marker_byte = *data.get(pos).ok_or(DecodeError::Malformed)?;
        pos += 1;
        let marker = Marker::from_u8(marker_byte);

        // For containers, how many child values we must descend into.
        let children: u64 = match marker {
            Marker::FixPos(_) | Marker::FixNeg(_) | Marker::Null | Marker::True | Marker::False => {
                0
            }
            Marker::U8 | Marker::I8 => {
                skip(data, &mut pos, 1)?;
                0
            }
            Marker::U16 | Marker::I16 => {
                skip(data, &mut pos, 2)?;
                0
            }
            Marker::U32 | Marker::I32 | Marker::F32 => {
                skip(data, &mut pos, 4)?;
                0
            }
            Marker::U64 | Marker::I64 | Marker::F64 => {
                skip(data, &mut pos, 8)?;
                0
            }
            Marker::FixStr(n) => {
                skip(data, &mut pos, u64::from(n))?;
                0
            }
            Marker::Str8 | Marker::Bin8 => {
                let len = read_uint(data, &mut pos, 1)?;
                skip(data, &mut pos, len)?;
                0
            }
            Marker::Str16 | Marker::Bin16 => {
                let len = read_uint(data, &mut pos, 2)?;
                skip(data, &mut pos, len)?;
                0
            }
            Marker::Str32 | Marker::Bin32 => {
                let len = read_uint(data, &mut pos, 4)?;
                skip(data, &mut pos, len)?;
                0
            }
            // ext = 1 type byte + payload.
            Marker::FixExt1 => {
                skip(data, &mut pos, 1 + 1)?;
                0
            }
            Marker::FixExt2 => {
                skip(data, &mut pos, 1 + 2)?;
                0
            }
            Marker::FixExt4 => {
                skip(data, &mut pos, 1 + 4)?;
                0
            }
            Marker::FixExt8 => {
                skip(data, &mut pos, 1 + 8)?;
                0
            }
            Marker::FixExt16 => {
                skip(data, &mut pos, 1 + 16)?;
                0
            }
            Marker::Ext8 => {
                let len = read_uint(data, &mut pos, 1)?;
                skip(data, &mut pos, len + 1)?;
                0
            }
            Marker::Ext16 => {
                let len = read_uint(data, &mut pos, 2)?;
                skip(data, &mut pos, len + 1)?;
                0
            }
            Marker::Ext32 => {
                let len = read_uint(data, &mut pos, 4)?;
                skip(data, &mut pos, len + 1)?;
                0
            }
            Marker::FixArray(n) => u64::from(n),
            Marker::Array16 => read_uint(data, &mut pos, 2)?,
            Marker::Array32 => read_uint(data, &mut pos, 4)?,
            // Each map entry is a key + a value, so two child values per pair.
            Marker::FixMap(n) => u64::from(n) * 2,
            Marker::Map16 => read_uint(data, &mut pos, 2)? * 2,
            Marker::Map32 => read_uint(data, &mut pos, 4)? * 2,
            Marker::Reserved => return Err(DecodeError::Malformed),
        };

        if children > 0 {
            // Opening a new container increases nesting by one level. Reject
            // before descending so the recursive decoder is never handed a
            // frame deeper than it can safely process.
            if stack.len() + 1 > max_depth {
                return Err(DecodeError::TooDeep);
            }
            stack.push(pending);
            pending = children;
        }
    }
}

/// Depth-checks `data` and then decodes it into `T` with `rmp_serde`.
///
/// The pre-scan guarantees `rmp_serde`'s recursive deserialization can never be
/// driven deeper than [`MAX_DECODE_DEPTH`], so a malicious deeply-nested frame is
/// rejected as `Err(TooDeep)` instead of overflowing the stack and aborting the
/// process. Callers should treat any `Err` as "drop this frame" (log + continue),
/// never panic.
///
/// # Errors
///
/// Returns [`DecodeError::TooDeep`] or [`DecodeError::Malformed`] if the pre-scan
/// rejects the frame, or [`DecodeError::Decode`] if `rmp_serde` fails to
/// deserialize a structurally-valid, in-depth frame into `T`.
pub fn decode_depth_checked<T: DeserializeOwned>(data: &[u8]) -> Result<T, DecodeError> {
    check_msgpack_depth(data, MAX_DECODE_DEPTH)?;
    rmp_serde::from_slice(data).map_err(DecodeError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::{Message as TopGunMessage, PingData};

    /// Builds `depth`-deep nested 1-element arrays (`0x91` repeated) terminated
    /// by nil (`0xc0`): a `depth + 1`-byte payload forcing `depth`-deep recursion.
    /// This is the exact shape from the audit repro harness.
    fn nested_msgpack(depth: usize) -> Vec<u8> {
        let mut buf = vec![0x91u8; depth];
        buf.push(0xc0);
        buf
    }

    #[test]
    fn shallow_nesting_passes_depth_check() {
        // Negative control: 64-deep is valid `MsgPack` and within the limit.
        assert!(check_msgpack_depth(&nested_msgpack(64), MAX_DECODE_DEPTH).is_ok());
    }

    #[test]
    fn at_limit_passes_over_limit_rejected() {
        assert!(check_msgpack_depth(&nested_msgpack(MAX_DECODE_DEPTH), MAX_DECODE_DEPTH).is_ok());
        assert!(matches!(
            check_msgpack_depth(&nested_msgpack(MAX_DECODE_DEPTH + 1), MAX_DECODE_DEPTH),
            Err(DecodeError::TooDeep)
        ));
    }

    #[test]
    fn deeply_nested_frame_rejected_not_overflowed() {
        // The DoS payload: ~512k-deep. The OLD path fed this straight to
        // `rmp_serde::from_slice`, recursing ~512k native frames → stack overflow
        // → SIGABRT. The pre-scan rejects it in O(MAX_DECODE_DEPTH) memory with
        // zero recursion, so this test returns an Err instead of aborting.
        let bytes = nested_msgpack(512_000);
        assert!(matches!(
            check_msgpack_depth(&bytes, MAX_DECODE_DEPTH),
            Err(DecodeError::TooDeep)
        ));
    }

    #[test]
    fn decode_depth_checked_rejects_deep_topgun_message() {
        // End-to-end: the exact call Phase 1 makes pre-auth. A deep frame must
        // come back as Err (dropped), never panic/abort.
        let bytes = nested_msgpack(512_000);
        let result: Result<TopGunMessage, _> = decode_depth_checked(&bytes);
        assert!(matches!(result, Err(DecodeError::TooDeep)));
    }

    #[test]
    fn deep_nesting_inside_map_value_rejected() {
        // Realistic attack shape: a 1-entry map whose value is deeply nested,
        // mimicking an operation envelope carrying a hostile `record`/`where`.
        // fixmap(1) + fixstr "k" + <deep array chain>.
        let mut bytes = vec![0x81u8, 0xa1, b'k'];
        bytes.extend(std::iter::repeat_n(0x91u8, 512_000));
        bytes.push(0xc0);
        assert!(matches!(
            check_msgpack_depth(&bytes, MAX_DECODE_DEPTH),
            Err(DecodeError::TooDeep)
        ));
    }

    #[test]
    fn truncated_frame_is_malformed_not_panic() {
        // Array claims 4 elements but the stream ends — must be a graceful Err.
        assert!(matches!(
            check_msgpack_depth(&[0x94], MAX_DECODE_DEPTH),
            Err(DecodeError::Malformed)
        ));
        // str8 claims 200 bytes that aren't there.
        assert!(matches!(
            check_msgpack_depth(&[0xd9, 0xc8, 0x00], MAX_DECODE_DEPTH),
            Err(DecodeError::Malformed)
        ));
        // Empty input.
        assert!(matches!(
            check_msgpack_depth(&[], MAX_DECODE_DEPTH),
            Err(DecodeError::Malformed)
        ));
        // Reserved marker 0xc1.
        assert!(matches!(
            check_msgpack_depth(&[0xc1], MAX_DECODE_DEPTH),
            Err(DecodeError::Malformed)
        ));
    }

    #[test]
    fn wide_but_shallow_frame_passes() {
        // A flat array of 1000 small ints is wide, not deep — must pass.
        let mut bytes = vec![0xdd, 0x00, 0x00, 0x03, 0xe8]; // array32, len 1000
        bytes.extend(std::iter::repeat_n(0x01u8, 1000));
        assert!(check_msgpack_depth(&bytes, MAX_DECODE_DEPTH).is_ok());
    }

    #[test]
    fn scalars_and_strings_advance_correctly() {
        // map { "a": 1, "b": "hi" } — exercises fixmap/fixstr/fixint cursor math.
        let bytes = vec![0x82, 0xa1, b'a', 0x01, 0xa1, b'b', 0xa2, b'h', b'i'];
        assert!(check_msgpack_depth(&bytes, MAX_DECODE_DEPTH).is_ok());
    }

    #[test]
    fn valid_message_round_trips_through_decode_depth_checked() {
        // A real, shallow TopGunMessage must still decode normally.
        let msg = TopGunMessage::Ping(PingData { timestamp: 0 });
        let bytes = rmp_serde::to_vec_named(&msg).expect("encode");
        let decoded: TopGunMessage = decode_depth_checked(&bytes).expect("decode");
        assert!(matches!(decoded, TopGunMessage::Ping(_)));
    }
}
