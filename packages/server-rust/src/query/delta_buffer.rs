//! Per-subscription delta accumulator for live query snapshot transitions.
//!
//! [`DeltaBuffer`] is activated immediately before a full-scan snapshot begins and
//! deactivated (drained) immediately after.  Any mutation that arrives on a key
//! *between* those two moments is captured so the snapshot can be corrected before
//! it is sent to the subscriber — eliminating torn-read windows caused by concurrent
//! writes during a multi-batch scan.
//!
//! The buffer is single-use: activate → (mutations land via `route`) → drain.
//! Concurrent mutations that exceed `capacity` put the buffer into an overflow state;
//! `deactivate_and_drain` then returns `Err(())` and the caller must fall back to a
//! retry (the snapshot is still consistent, just stale for those keys).

use std::collections::HashMap;

use parking_lot::Mutex;
use topgun_core::hlc::Timestamp;

// ---------------------------------------------------------------------------
// Typed error-code constants (exported via query/mod.rs)
// ---------------------------------------------------------------------------

/// Returned in `QueryRespPayload::code` when a subscription requests a sort that
/// cannot be bounded (no `limit` + sort fields that require a full scan each tick).
pub const QUERY_UNBOUNDED_SORT: &str = "QUERY_UNBOUNDED_SORT";

/// Returned in `QueryRespPayload::code` when the delta buffer overflows during a
/// snapshot — too many concurrent mutations arrived while the scan was in flight.
pub const QUERY_SNAPSHOT_OVERFLOW: &str = "QUERY_SNAPSHOT_OVERFLOW";

// ---------------------------------------------------------------------------
// DeltaEntry
// ---------------------------------------------------------------------------

/// A single per-key entry captured by the buffer while the snapshot scan is in flight.
// Fields are read by the live-query snapshot correction path wired in G3.
#[allow(dead_code)]
pub(crate) struct DeltaEntry {
    /// The record value at the time the mutation was routed.
    pub(crate) value: rmpv::Value,
    /// Timestamp of the mutation; used to fence against mutations that pre-date the scan.
    pub(crate) timestamp: Timestamp,
    /// Whether the record matched the subscription predicate at the time of mutation.
    pub(crate) matches: bool,
}

// ---------------------------------------------------------------------------
// DeltaBuffer
// ---------------------------------------------------------------------------

/// Internal buffer state, protected by a single mutex.
struct Inner {
    /// Whether the buffer is currently accepting mutations.
    active: bool,
    /// Accumulated per-key mutations; newest write wins when the same key is routed twice.
    entries: HashMap<String, DeltaEntry>,
    /// Set to `true` when `entries.len()` exceeded `capacity`; triggers `Err(())` on drain.
    overflowed: bool,
}

/// A capacity-bounded, activation-gated per-key delta accumulator.
///
/// Typical lifecycle:
/// 1. `activate()` — open the buffer just before the first scan batch starts.
/// 2. Concurrent mutations call `route()` for each affected key.
/// 3. `deactivate_and_drain(fence)` — close the buffer, discard entries whose timestamp
///    is ≤ the fence (they were already captured in the scan), return the rest.
// The full usage (activate + deactivate_and_drain) is wired in G3's live-query
// subscription handler.
#[allow(dead_code)]
pub struct DeltaBuffer {
    inner: Mutex<Inner>,
    /// Maximum number of distinct keys the buffer will accept before overflowing.
    capacity: usize,
}

impl DeltaBuffer {
    /// Create a new, inactive buffer with the given key capacity.
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

    /// Record a mutation for `key`.
    ///
    /// If the buffer is not active the call is silently ignored — mutations that
    /// arrive outside the activation window are already visible to the scan or
    /// will be pushed via the normal live-delta path.
    ///
    /// If recording this key would exceed `capacity` the buffer is marked as
    /// overflowed; subsequent `route` calls are still no-ops (no additional state
    /// is modified once overflow is set).
    pub fn route(&self, key: &str, value: rmpv::Value, timestamp: Timestamp, matches: bool) {
        let mut inner = self.inner.lock();
        if !inner.active {
            return;
        }
        if inner.overflowed {
            return;
        }
        // Check capacity before inserting a new key; existing keys update in-place.
        if !inner.entries.contains_key(key) && inner.entries.len() >= self.capacity {
            inner.overflowed = true;
            return;
        }
        // Per-key LWW: only update if the incoming timestamp is strictly newer than
        // what we already have. This ensures the snapshot correction uses the latest
        // known value, not an accidentally replayed older write.
        let should_insert = match inner.entries.get(key) {
            None => true,
            Some(existing) => timestamp > existing.timestamp,
        };
        if should_insert {
            inner.entries.insert(
                key.to_string(),
                DeltaEntry {
                    value,
                    timestamp,
                    matches,
                },
            );
        }
    }

    /// Close the buffer and return all entries whose timestamp is strictly after `fence`.
    ///
    /// Entries at or before `fence` were already included in the scan snapshot and must
    /// not be replayed.
    ///
    /// Returns:
    /// - `Ok(vec)` — list of `(key, DeltaEntry)` pairs to overlay on the snapshot.
    /// - `Err(())` — the buffer overflowed; caller must retry the snapshot.
    ///
    /// The deactivation and drain happen under a single lock acquisition so no mutation
    /// can slip in between closing the window and reading the accumulated set.
    // Called by the live-query snapshot correction path wired in G3.
    #[allow(dead_code)]
    pub(crate) fn deactivate_and_drain(
        &self,
        fence: Timestamp,
    ) -> Result<Vec<(String, DeltaEntry)>, ()> {
        let mut inner = self.inner.lock();
        inner.active = false;
        if inner.overflowed {
            return Err(());
        }
        let drained: Vec<(String, DeltaEntry)> = inner
            .entries
            .drain()
            .filter(|(_, entry)| entry.timestamp > fence)
            .collect();
        Ok(drained)
    }
}
