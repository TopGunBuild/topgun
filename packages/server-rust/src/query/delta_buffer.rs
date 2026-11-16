//! Per-subscription snapshot buffer of a live query subscription (TG-LIVE-001).
//!
//! A live subscribe activates its [`DeltaBuffer`] before the subscription is
//! registered, so every fan-out that finds the registration finds a capturing
//! buffer. While the buffer is active, the live fan-out records each write in
//! it instead of sending a `QUERY_UPDATE`; when the scan is done, the handler
//! closes the buffer under its lock, folds what was captured into the snapshot
//! and enqueues the snapshot itself, so the snapshot precedes every update of
//! the registration.
//!
//! What is captured is the event's routed value, whatever the event kind: an
//! LWW put routes its stored record, while an LWW remove, an expiry remove and
//! an OR add or remove route no record or a value-less one, which the fold
//! treats as "no row" — what the live path does with them too. The last
//! arrival per key wins, because under the key's writer the last arrival is
//! what the store holds; no stamp is compared. The fold checks each captured
//! record against TG-TTL-001 at fold time, so a value that expired while it
//! was buffered is folded as "no row", as the scan would have left it out.
//!
//! The bound counts distinct keys of any kind of event: a key routed again
//! never counts twice. Once a key not yet recorded would exceed the capacity,
//! the buffer overflows and the handler answers with the snapshot and
//! `QUERY_SNAPSHOT_OVERFLOW` instead of folding.

use std::collections::HashMap;

use parking_lot::Mutex;
use topgun_core::LWWRecord;

// ---------------------------------------------------------------------------
// Typed error-code constants (exported via query/mod.rs)
// ---------------------------------------------------------------------------

/// Returned in `QueryRespPayload::code` when a subscription requests a sort that
/// cannot be bounded (no `limit` + sort fields that require a full scan each tick).
pub const QUERY_UNBOUNDED_SORT: &str = "QUERY_UNBOUNDED_SORT";

/// Returned in `QueryRespPayload::code` when a live subscription's snapshot
/// could not carry every write made during its scan — too many distinct keys
/// were written, or the connection had no free slot for the snapshot. It
/// follows the snapshot, and the server has dropped the subscription.
pub const QUERY_SNAPSHOT_OVERFLOW: &str = "QUERY_SNAPSHOT_OVERFLOW";

// ---------------------------------------------------------------------------
// DeltaEntry
// ---------------------------------------------------------------------------

/// The last write routed for one key while the buffer was active.
pub(crate) struct DeltaEntry {
    /// The routed record: the stored record of an LWW put, else `None` or a
    /// value-less record, both meaning "no row".
    pub(crate) record: Option<LWWRecord<rmpv::Value>>,
    /// Whether the routed value matched the subscription's predicate.
    pub(crate) matches: bool,
}

/// What the buffer holds when it is closed.
pub(crate) enum Captured {
    /// Every distinct key routed while active, each with its last arrival.
    Entries(Vec<(String, DeltaEntry)>),
    /// More distinct keys were routed than the capacity allows.
    Overflowed,
}

/// How [`DeltaBuffer::close`] leaves the buffer.
pub(crate) enum Closing {
    /// The snapshot was delivered: later writes go to the live path.
    Live,
    /// Nothing was delivered: the buffer keeps owning every later write and
    /// records none of them, so the abandoned registration sends nothing.
    Sealed,
}

// ---------------------------------------------------------------------------
// DeltaBuffer
// ---------------------------------------------------------------------------

/// Internal buffer state, protected by a single mutex.
struct Inner {
    /// Whether the buffer owns routed writes.
    active: bool,
    /// The last arrival per distinct key routed while active.
    entries: HashMap<String, DeltaEntry>,
    /// Set once the distinct keys would exceed `capacity`; an overflowed
    /// buffer records nothing more. Active and overflowed with no entries is
    /// the sealed state.
    overflowed: bool,
}

/// A capacity-bounded, activation-gated per-key snapshot buffer.
///
/// Lifecycle: `activate()` before the subscription is registered; writes call
/// `route()`; the subscribe handler calls `close()` once, or `seal()` when it
/// gives up before delivering.
pub struct DeltaBuffer {
    inner: Mutex<Inner>,
    /// Maximum number of distinct keys the buffer will accept before overflowing.
    capacity: usize,
}

impl DeltaBuffer {
    /// Create a new, inactive buffer with the given key capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                active: false,
                entries: HashMap::new(),
                overflowed: false,
            }),
            capacity,
        }
    }

    /// Open the buffer so that subsequent `route` calls are recorded.
    ///
    /// Calling `activate` on an already-active buffer is a no-op (idempotent).
    pub fn activate(&self) {
        let mut inner = self.inner.lock();
        inner.active = true;
    }

    /// Returns `true` while the buffer owns routed writes: from `activate`
    /// until `close` returns [`Closing::Live`], and for ever once sealed.
    pub fn is_active(&self) -> bool {
        self.inner.lock().active
    }

    /// Record a write of `key`.
    ///
    /// Returns `true` when the buffer was active and therefore OWNS this write:
    /// the caller must not also send it live. Returns `false` when the buffer is
    /// inactive and the caller sends it through the live path.
    ///
    /// The active check and the record happen under one lock acquisition, the
    /// lock `close` holds while the snapshot is enqueued, so a write is either
    /// in the folded snapshot or sent live after it, never both and never
    /// before it.
    ///
    /// `record` is cloned only when it is recorded. The last arrival of a key
    /// replaces what was recorded for it. A key not yet recorded that would
    /// make the distinct keys exceed `capacity` overflows the buffer; an
    /// overflowed buffer records nothing more but still owns every write.
    pub fn route(&self, key: &str, record: Option<&LWWRecord<rmpv::Value>>, matches: bool) -> bool {
        let mut inner = self.inner.lock();
        if !inner.active {
            return false;
        }
        if inner.overflowed {
            return true;
        }
        if !inner.entries.contains_key(key) && inner.entries.len() >= self.capacity {
            inner.overflowed = true;
            inner.entries.clear();
            return true;
        }
        inner.entries.insert(
            key.to_string(),
            DeltaEntry {
                record: record.cloned(),
                matches,
            },
        );
        true
    }

    /// Closes the buffer: runs `deliver` with what was captured, under the
    /// buffer lock, so no write can be routed between the capture and the
    /// delivery. [`Closing::Live`] makes the buffer inactive and empty;
    /// [`Closing::Sealed`] leaves it owning every later write and recording
    /// none.
    ///
    /// `deliver` must not block or await: every writer of the subscription's
    /// map waits on this lock. It may take a registry lock (the buffer lock
    /// may be taken before a registry lock, never after).
    pub(crate) fn close(&self, deliver: impl FnOnce(Captured) -> Closing) {
        let mut inner = self.inner.lock();
        let captured = if inner.overflowed {
            Captured::Overflowed
        } else {
            Captured::Entries(inner.entries.drain().collect())
        };
        match deliver(captured) {
            Closing::Live => {
                inner.active = false;
                inner.overflowed = false;
                inner.entries.clear();
            }
            Closing::Sealed => Self::seal_locked(&mut inner),
        }
    }

    /// Seals the buffer: it owns every later write and records none of them.
    /// Used when a subscribe gives up before delivering its snapshot.
    pub(crate) fn seal(&self) {
        Self::seal_locked(&mut self.inner.lock());
    }

    fn seal_locked(inner: &mut Inner) {
        inner.active = true;
        inner.overflowed = true;
        inner.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::hlc::Timestamp;

    fn rec(n: i64) -> LWWRecord<rmpv::Value> {
        LWWRecord {
            value: Some(rmpv::Value::Integer(n.into())),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "n1".to_string(),
            },
            ttl_ms: None,
        }
    }

    /// Closes the buffer as delivered and returns what it captured.
    fn close_live(buf: &DeltaBuffer) -> Option<HashMap<String, DeltaEntry>> {
        let mut out = None;
        buf.close(|captured| {
            if let Captured::Entries(entries) = captured {
                out = Some(entries.into_iter().collect());
            }
            Closing::Live
        });
        out
    }

    fn value_of(entry: &DeltaEntry) -> Option<rmpv::Value> {
        entry.record.as_ref().and_then(|r| r.value.clone())
    }

    #[test]
    fn the_last_arrival_of_a_key_wins_whatever_its_stamp() {
        let buf = DeltaBuffer::new(16);
        buf.activate();
        let mut newer = rec(1);
        newer.timestamp.millis = 500;
        assert!(buf.route("k", Some(&newer), true));
        // A later arrival with a lower stamp still replaces it: under the key
        // writer the store keeps the last arrival.
        assert!(buf.route("k", Some(&rec(2)), false));

        let entries = close_live(&buf).expect("no overflow");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            value_of(&entries["k"]),
            Some(rmpv::Value::Integer(2.into()))
        );
        assert!(!entries["k"].matches);
    }

    #[test]
    fn a_none_event_is_recorded_and_replaces_an_earlier_value() {
        let buf = DeltaBuffer::new(16);
        buf.activate();
        assert!(buf.route("k", Some(&rec(1)), true));
        assert!(buf.route("k", None, false));

        let entries = close_live(&buf).expect("no overflow");
        assert!(entries["k"].record.is_none());
        assert!(!entries["k"].matches);
    }

    #[test]
    fn the_bound_counts_distinct_keys_of_any_event_kind() {
        // A key routed again never counts twice, whatever its event kind.
        let one = DeltaBuffer::new(1);
        one.activate();
        for i in 0..10 {
            let record = rec(i);
            assert!(one.route("k", (i % 2 == 0).then_some(&record), i % 2 == 0));
        }
        assert!(
            close_live(&one).is_some(),
            "one key must not overflow capacity 1"
        );

        // Capacity + 1 distinct keys overflow, removes and values alike.
        let two = DeltaBuffer::new(2);
        two.activate();
        assert!(two.route("a", Some(&rec(1)), true));
        assert!(two.route("b", None, false));
        assert!(two.route("c", None, false));
        let mut overflowed = false;
        two.close(|captured| {
            overflowed = matches!(captured, Captured::Overflowed);
            Closing::Sealed
        });
        assert!(overflowed);
    }

    #[test]
    fn a_sealed_buffer_owns_every_write_and_records_none() {
        let buf = DeltaBuffer::new(16);
        buf.activate();
        assert!(buf.route("a", Some(&rec(1)), true));
        buf.seal();
        assert!(buf.route("b", Some(&rec(2)), true));
        assert!(buf.is_active());
        let mut overflowed = false;
        buf.close(|captured| {
            overflowed = matches!(captured, Captured::Overflowed);
            Closing::Sealed
        });
        assert!(overflowed, "a sealed buffer holds nothing to fold");
        assert!(buf.route("c", None, false), "it still owns later writes");
    }

    #[test]
    fn close_with_live_reopens_the_live_path() {
        let buf = DeltaBuffer::new(16);
        // Inactive: the caller sends live.
        assert!(!buf.route("k", Some(&rec(1)), true));
        buf.activate();
        assert!(buf.route("k", Some(&rec(2)), true));
        let entries = close_live(&buf).expect("no overflow");
        assert_eq!(entries.len(), 1);
        assert!(!buf.is_active());
        assert!(!buf.route("k", Some(&rec(3)), true));
        assert!(close_live(&buf).expect("no overflow").is_empty());
    }
}
