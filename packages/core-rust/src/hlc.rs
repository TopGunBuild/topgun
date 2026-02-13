//! Hybrid Logical Clock (HLC) for distributed causality tracking.
//!
//! Provides monotonically increasing timestamps that combine physical wall-clock
//! time with a logical counter, ensuring total ordering of events across nodes.
//! The HLC is the foundation for LWW (Last-Write-Wins) conflict resolution.
//!
//! # Wire format
//!
//! Timestamps serialize to the string format `"millis:counter:nodeId"` for
//! cross-language compatibility with the TypeScript client. Node IDs must not
//! contain the `:` character.

use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A hybrid logical timestamp combining physical time, logical counter, and node identity.
///
/// Ordering is defined as: millis first, then counter, then node_id (lexicographic byte order).
/// This matches the TypeScript `HLC.compare()` behavior for ASCII-only node IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timestamp {
    /// Physical wall-clock milliseconds since Unix epoch.
    pub millis: u64,
    /// Logical counter for events within the same millisecond.
    pub counter: u32,
    /// Unique identifier of the node that generated this timestamp.
    pub node_id: String,
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.millis
            .cmp(&other.millis)
            .then_with(|| self.counter.cmp(&other.counter))
            .then_with(|| self.node_id.cmp(&other.node_id))
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Abstraction over the system clock for dependency injection.
///
/// Allows deterministic testing by replacing the real clock with a virtual one.
/// The default implementation ([`SystemClock`]) delegates to `std::time::SystemTime`.
pub trait ClockSource: Send + Sync {
    /// Returns the current time as milliseconds since Unix epoch.
    fn now(&self) -> u64;
}

/// Default clock source that reads the real system time.
#[derive(Debug, Clone)]
pub struct SystemClock;

impl ClockSource for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_millis() as u64
    }
}

/// Hybrid Logical Clock implementation.
///
/// Generates monotonically increasing [`Timestamp`]s by combining wall-clock time
/// with a logical counter. When the system clock advances, the counter resets to 0.
/// When the system clock is unchanged or behind, the counter increments.
///
/// # Drift detection
///
/// When merging a remote timestamp via [`HLC::update`], the clock detects if the
/// remote time exceeds the local time by more than `max_drift_ms`. In strict mode
/// this returns an error; otherwise a warning is logged via the `tracing` crate.
pub struct HLC {
    last_millis: u64,
    last_counter: u32,
    node_id: String,
    strict_mode: bool,
    max_drift_ms: u64,
    clock_source: Box<dyn ClockSource>,
}

/// A Last-Write-Wins record wrapping a value with its causal timestamp.
///
/// Generic over `V` so that any serializable type can be stored, not just [`Value`](crate::Value).
/// The `ttl_ms` field enables time-to-live expiration checked against the HLC clock source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "V: Serialize",
    deserialize = "V: serde::de::DeserializeOwned"
))]
pub struct LWWRecord<V> {
    /// The stored value, or `None` if this record represents a tombstone (deletion).
    pub value: Option<V>,
    /// Causal timestamp assigned by the writing node's HLC.
    pub timestamp: Timestamp,
    /// Optional time-to-live in milliseconds. Checked against `HLC::clock_source().now()`.
    pub ttl_ms: Option<u64>,
}

/// An Observed-Remove Map record associating a value with a unique tag.
///
/// Each concurrent addition to an ORMap entry gets a unique `tag` (typically
/// `"millis:counter:nodeId"`). Removals target specific tags, allowing concurrent
/// adds and removes to be resolved without lost updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "V: Serialize",
    deserialize = "V: serde::de::DeserializeOwned"
))]
pub struct ORMapRecord<V> {
    /// The stored value.
    pub value: V,
    /// Causal timestamp assigned by the writing node's HLC.
    pub timestamp: Timestamp,
    /// Unique tag identifying this particular addition (typically `"millis:counter:nodeId"`).
    pub tag: String,
    /// Optional time-to-live in milliseconds.
    pub ttl_ms: Option<u64>,
}

/// Result of merging a key in a CRDT map, reporting how many entries were added or updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeKeyResult {
    /// Number of new entries added during the merge.
    pub added: usize,
    /// Number of existing entries updated during the merge.
    pub updated: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_ordering_millis_first() {
        let a = Timestamp { millis: 100, counter: 5, node_id: "z".to_string() };
        let b = Timestamp { millis: 200, counter: 0, node_id: "a".to_string() };
        assert!(a < b);
    }

    #[test]
    fn timestamp_ordering_counter_second() {
        let a = Timestamp { millis: 100, counter: 1, node_id: "z".to_string() };
        let b = Timestamp { millis: 100, counter: 2, node_id: "a".to_string() };
        assert!(a < b);
    }

    #[test]
    fn timestamp_ordering_node_id_third() {
        let a = Timestamp { millis: 100, counter: 1, node_id: "a".to_string() };
        let b = Timestamp { millis: 100, counter: 1, node_id: "b".to_string() };
        assert!(a < b);
    }

    #[test]
    fn timestamp_equal() {
        let a = Timestamp { millis: 100, counter: 1, node_id: "node".to_string() };
        let b = Timestamp { millis: 100, counter: 1, node_id: "node".to_string() };
        assert_eq!(a.cmp(&b), Ordering::Equal);
    }

    #[test]
    fn system_clock_returns_nonzero() {
        let clock = SystemClock;
        assert!(clock.now() > 0);
    }

    #[test]
    fn merge_key_result_equality() {
        let a = MergeKeyResult { added: 1, updated: 2 };
        let b = MergeKeyResult { added: 1, updated: 2 };
        assert_eq!(a, b);
    }

    #[test]
    fn timestamp_serde_roundtrip() {
        let ts = Timestamp {
            millis: 1_700_000_000_000,
            counter: 42,
            node_id: "node-abc".to_string(),
        };
        let bytes = rmp_serde::to_vec(&ts).expect("serialize");
        let decoded: Timestamp = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(ts, decoded);
    }
}
