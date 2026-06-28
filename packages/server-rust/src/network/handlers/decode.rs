//! Depth- and structure-bounded `MsgPack` decoding for untrusted inbound frames.
//!
//! Every inbound WebSocket frame (and the HTTP `/sync` body) is `MsgPack`, decoded
//! with `rmp_serde` — and on `/ws` the decode happens in Phase 1, BEFORE the
//! connection is authenticated. The message types are internally tagged
//! (`#[serde(tag = "type")]`, which buffers frame content into serde's recursive
//! `Content` type) and embed `rmpv::Value` — itself a recursive enum — so in
//! principle a deeply-nested frame drives one native stack frame per nesting level
//! during deserialization. An unbounded recursive decode would overflow the stack,
//! and a stack overflow in safe Rust aborts the whole process (uncatchable: no
//! `catch_unwind`, no per-task isolation) — one small unauthenticated frame could
//! kill every connection on the node.
//!
//! Note the "unbounded" is no longer literally true of the pinned codec:
//! `rmp_serde` 1.3.1 caps its OWN recursion at 1024 levels (an internal
//! `depth_count!` guard, default `depth: 1024`) and returns `DepthLimitExceeded`
//! instead of overflowing. That ceiling is a safety net, but it is an unstable
//! implementation detail — not a documented API contract — and it would not exist
//! on a different codec we might swap to. [`decode_depth_checked`] therefore
//! enforces our OWN bound, version-independently, by scanning the raw `MsgPack`
//! bytes iteratively — an explicit heap stack, so the scanner itself never
//! recurses and cannot overflow — and rejecting any frame whose container nesting
//! exceeds [`MAX_DECODE_DEPTH`] before the bytes ever reach `rmp_serde`.
//! [`MAX_DECODE_DEPTH`] sits well below the dependency's 1024 ceiling, so our
//! reject fires first and predictably, independent of codec internals. Paired with
//! the inbound frame-size cap on the WebSocket upgrade, this bounds the decoder's
//! input to well-formed, shallow frames. The pinning regression test
//! `rmp_serde_own_depth_limit_is_present` fails loudly if a future bump removes the
//! dependency's safety net, re-escalating the residual risk.

use rmp::Marker;
use serde::de::DeserializeOwned;

/// Maximum container-nesting depth accepted for an inbound `MsgPack` frame.
///
/// `TopGun` envelopes themselves are shallow, but the payload they carry
/// (`rmpv::Value` records and `where` predicates) is user-controlled and may be
/// machine-generated — nested JSON-like documents can run deeper than a typical
/// hand-written object. Measured against representative payloads, typical
/// envelopes nest ≤9 levels and a deliberately 40-deep machine-generated record
/// value reaches ~43; this bound is set to 256, leaving a wide margin above any
/// observed legitimate payload while staying well below `rmp_serde`'s own 1024
/// ceiling (so our reject fires first, predictably) and far below anything that
/// could exhaust a worker stack. Because production traffic depth is ultimately
/// unknowable from fixtures, frames deeper than [`WARN_DECODE_DEPTH`] but still
/// accepted are logged (see [`decode_depth_checked`]) so the real-world
/// distribution can be observed before this bound is ever tightened — telemetry,
/// not a guess.
pub const MAX_DECODE_DEPTH: usize = 256;

/// Soft telemetry threshold. An accepted frame nesting deeper than this is
/// unusual — it would have been rejected by the original, tighter 128-level bound
/// — but it is still well within [`MAX_DECODE_DEPTH`], so it is logged at `debug`
/// rather than rejected. This is how the real-world depth distribution is gathered
/// instead of guessed; if logs show legitimate traffic approaching the cap, raise
/// [`MAX_DECODE_DEPTH`] with data, don't tighten blind.
pub const WARN_DECODE_DEPTH: usize = 128;

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
/// On success returns the **maximum container-nesting depth observed** (0 for a
/// flat scalar), which the caller uses for depth telemetry. Returns `Err(TooDeep)`
/// the moment nesting would exceed `max_depth`, and `Err(Malformed)` on a truncated
/// stream or reserved marker. The scan uses an explicit heap stack and **never
/// recurses**, so it cannot itself overflow on adversarial input — that is the
/// whole point: it runs before the recursive `rmp_serde` decoder and rejects frames
/// that would overflow it.
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
pub fn check_msgpack_depth(data: &[u8], max_depth: usize) -> Result<usize, DecodeError> {
    let mut pos = 0usize;

    // `stack[i]` = number of child values still to read at nesting level `i`.
    // `pending` is the count at the current (deepest open) level. The scanner
    // expects exactly one top-level value. `max_observed` tracks the deepest
    // nesting reached, for the caller's telemetry.
    let mut stack: Vec<u64> = Vec::new();
    let mut pending: u64 = 1;
    let mut max_observed: usize = 0;

    loop {
        // Pop levels whose child count has been fully consumed.
        while pending == 0 {
            match stack.pop() {
                Some(parent_remaining) => pending = parent_remaining,
                None => return Ok(max_observed), // all values consumed
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
            max_observed = max_observed.max(stack.len());
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
    let observed = check_msgpack_depth(data, MAX_DECODE_DEPTH)?;
    // Telemetry, not a guard: an accepted frame deeper than the old 128 bound is
    // unusual. Log it (without rejecting) so the real-world depth distribution is
    // measured before MAX_DECODE_DEPTH is ever tightened on a guess.
    if observed > WARN_DECODE_DEPTH {
        tracing::debug!(
            depth = observed,
            limit = MAX_DECODE_DEPTH,
            "accepted unusually deeply-nested inbound MsgPack frame"
        );
    }
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
        // The DoS-shaped payload: ~512k-deep. The pre-scan rejects it in
        // O(MAX_DECODE_DEPTH) memory with zero recursion, returning Err instead of
        // descending. (The pinned `rmp_serde` would itself cap at 1024 and error,
        // not overflow — but the pre-scan is the version-independent guard that does
        // not depend on that codec internal; see `rmp_serde_own_depth_limit_is_present`.)
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

    #[test]
    fn scanner_rejects_within_rmp_serde_own_limit() {
        // DISCRIMINATOR — proves OUR bound is active and strictly tighter than the
        // dependency's. A frame nested deeper than MAX_DECODE_DEPTH (256) but well
        // within `rmp_serde`'s own 1024 ceiling: the pre-scan MUST reject it, while
        // raw `rmp_serde` decodes the very same bytes WITHOUT error. If
        // MAX_DECODE_DEPTH were raised to/above this depth, or the pre-scan removed,
        // this fails — so a green here is not a property of the codec, it is our fix.
        let depth = 500; // 256 < 500 < 1024
        let bytes = nested_msgpack(depth);
        assert!(
            matches!(
                check_msgpack_depth(&bytes, MAX_DECODE_DEPTH),
                Err(DecodeError::TooDeep)
            ),
            "pre-scan must reject {depth}-deep (> MAX_DECODE_DEPTH={MAX_DECODE_DEPTH})"
        );
        let raw: Result<rmpv::Value, _> = rmp_serde::from_slice(&bytes);
        assert!(
            raw.is_ok(),
            "rmp_serde itself ACCEPTS {depth}-deep (< its 1024 limit) — proving our \
             reject is depth-driven and tighter than the dependency, got {raw:?}"
        );
    }

    #[test]
    fn rmp_serde_own_depth_limit_is_present() {
        // PINNING REGRESSION on the dependency safety net. `rmp_serde` 1.3.1 caps
        // recursion at 1024 and returns DepthLimitExceeded rather than overflowing
        // the stack. `decode_depth_checked` is the PRIMARY, version-independent
        // guard — but any future code path that bypasses the pre-scan still leans on
        // this ceiling to avoid a stack-overflow abort. If a dependency bump removes
        // it, this fails loudly so the residual risk is caught and re-escalated.
        let within: Result<rmpv::Value, _> = rmp_serde::from_slice(&nested_msgpack(1023));
        assert!(within.is_ok(), "1023-deep is within rmp_serde's 1024 limit");
        let over = rmp_serde::from_slice::<rmpv::Value>(&nested_msgpack(1025));
        assert!(
            matches!(over, Err(rmp_serde::decode::Error::DepthLimitExceeded)),
            "rmp_serde must still reject >1024-deep with DepthLimitExceeded, got {over:?}"
        );
    }

    #[test]
    fn check_reports_observed_depth() {
        // The scanner returns the deepest nesting it saw — this powers the WARN
        // telemetry in `decode_depth_checked`.
        assert_eq!(
            check_msgpack_depth(&nested_msgpack(10), MAX_DECODE_DEPTH).unwrap(),
            10
        );
        // A flat scalar is depth 0.
        assert_eq!(check_msgpack_depth(&[0xc0], MAX_DECODE_DEPTH).unwrap(), 0);
        // A frame at exactly the WARN threshold is reported (caller decides to log).
        assert_eq!(
            check_msgpack_depth(&nested_msgpack(WARN_DECODE_DEPTH + 1), MAX_DECODE_DEPTH).unwrap(),
            WARN_DECODE_DEPTH + 1
        );
    }
}
