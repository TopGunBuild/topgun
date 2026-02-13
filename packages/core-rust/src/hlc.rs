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
use std::fmt;
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

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.millis, self.counter, self.node_id)
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
///
/// # Examples
///
/// ```
/// use topgun_core::hlc::{HLC, SystemClock};
///
/// let mut hlc = HLC::new("node-1".to_string(), Box::new(SystemClock));
/// let ts1 = hlc.now();
/// let ts2 = hlc.now();
/// assert!(ts1 < ts2);
/// ```
pub struct HLC {
    last_millis: u64,
    last_counter: u32,
    node_id: String,
    strict_mode: bool,
    max_drift_ms: u64,
    clock_source: Box<dyn ClockSource>,
}

impl HLC {
    /// Creates a new HLC with the given node ID and clock source.
    ///
    /// Uses default options: non-strict mode, 60-second max drift.
    pub fn new(node_id: String, clock_source: Box<dyn ClockSource>) -> Self {
        Self {
            last_millis: 0,
            last_counter: 0,
            node_id,
            strict_mode: false,
            max_drift_ms: 60_000,
            clock_source,
        }
    }

    /// Creates a new HLC with explicit strict mode and max drift configuration.
    pub fn with_options(
        node_id: String,
        clock_source: Box<dyn ClockSource>,
        strict_mode: bool,
        max_drift_ms: u64,
    ) -> Self {
        Self {
            last_millis: 0,
            last_counter: 0,
            node_id,
            strict_mode,
            max_drift_ms,
            clock_source,
        }
    }

    /// Returns the node ID of this HLC instance.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Returns whether strict mode is enabled.
    pub fn strict_mode(&self) -> bool {
        self.strict_mode
    }

    /// Returns the maximum allowed clock drift in milliseconds.
    pub fn max_drift_ms(&self) -> u64 {
        self.max_drift_ms
    }

    /// Returns a reference to the clock source used by this HLC.
    ///
    /// Useful for LWWMap/ORMap to access the same clock for TTL checks.
    pub fn clock_source(&self) -> &dyn ClockSource {
        &*self.clock_source
    }

    /// Generates a new unique timestamp for a local event.
    ///
    /// Ensures monotonicity: always greater than any previously generated or received timestamp.
    /// If the system clock advances past the last logical time, the counter resets to 0.
    /// Otherwise the counter increments to maintain uniqueness.
    pub fn now(&mut self) -> Timestamp {
        let system_time = self.clock_source.now();

        if system_time > self.last_millis {
            // System clock advanced: reset counter
            self.last_millis = system_time;
            self.last_counter = 0;
        } else {
            // System clock unchanged or behind: increment counter
            self.last_counter += 1;
        }

        Timestamp {
            millis: self.last_millis,
            counter: self.last_counter,
            node_id: self.node_id.clone(),
        }
    }

    /// Updates the local clock based on a received remote timestamp.
    ///
    /// Must be called whenever a message/event is received from another node.
    /// Merges the remote timestamp with the local state to maintain causality.
    ///
    /// # Errors
    ///
    /// Returns an error in strict mode if the remote timestamp's millis exceeds
    /// `local_system_time + max_drift_ms`. In non-strict mode, a warning is logged
    /// but the timestamp is accepted (AP system behavior).
    pub fn update(&mut self, remote: &Timestamp) -> Result<(), String> {
        let system_time = self.clock_source.now();

        // Drift detection: only check positive drift (remote ahead of local)
        if remote.millis > system_time {
            let drift = remote.millis - system_time;
            if drift > self.max_drift_ms {
                if self.strict_mode {
                    return Err(format!(
                        "Clock drift detected: Remote time {} is {}ms ahead of local {} (threshold: {}ms)",
                        remote.millis, drift, system_time, self.max_drift_ms
                    ));
                }
                tracing::warn!(
                    drift = drift,
                    remote_millis = remote.millis,
                    local_millis = system_time,
                    max_drift_ms = self.max_drift_ms,
                    "Clock drift detected"
                );
            }
        }

        let max_millis = self.last_millis.max(system_time).max(remote.millis);

        if max_millis == self.last_millis && max_millis == remote.millis {
            // Both clocks on the same millisecond: take max counter + 1
            self.last_counter = self.last_counter.max(remote.counter) + 1;
        } else if max_millis == self.last_millis {
            // Local logical clock is ahead: just increment
            self.last_counter += 1;
        } else if max_millis == remote.millis {
            // Remote clock is ahead: fast-forward
            self.last_counter = remote.counter + 1;
        } else {
            // System time is ahead of both: reset counter
            self.last_counter = 0;
        }

        self.last_millis = max_millis;
        Ok(())
    }

    /// Compares two timestamps.
    ///
    /// Returns `Ordering::Less` if `a < b`, `Ordering::Greater` if `a > b`,
    /// `Ordering::Equal` if they are identical.
    ///
    /// Comparison order: millis first, then counter, then node_id (byte-order).
    pub fn compare(a: &Timestamp, b: &Timestamp) -> Ordering {
        a.cmp(b)
    }

    /// Serializes a timestamp to the wire format `"millis:counter:nodeId"`.
    pub fn to_string(ts: &Timestamp) -> String {
        ts.to_string()
    }

    /// Parses a timestamp from the wire format `"millis:counter:nodeId"`.
    ///
    /// # Errors
    ///
    /// Returns an error if the string does not contain exactly 3 colon-separated parts,
    /// or if millis/counter cannot be parsed as integers.
    pub fn parse(s: &str) -> Result<Timestamp, String> {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        if parts.len() != 3 {
            return Err(format!("Invalid timestamp format: {s}"));
        }

        let millis = parts[0]
            .parse::<u64>()
            .map_err(|e| format!("Invalid millis in timestamp: {e}"))?;
        let counter = parts[1]
            .parse::<u32>()
            .map_err(|e| format!("Invalid counter in timestamp: {e}"))?;
        let node_id = parts[2].to_string();

        if node_id.is_empty() {
            return Err(format!("Invalid timestamp format: {s}"));
        }

        Ok(Timestamp {
            millis,
            counter,
            node_id,
        })
    }
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
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::sync::Arc;

    use super::*;

    /// A deterministic clock source for testing. Time is controlled explicitly.
    struct FixedClock {
        time: Arc<AtomicU64>,
    }

    impl FixedClock {
        fn new(initial: u64) -> (Self, Arc<AtomicU64>) {
            let time = Arc::new(AtomicU64::new(initial));
            (Self { time: time.clone() }, time)
        }
    }

    impl ClockSource for FixedClock {
        fn now(&self) -> u64 {
            self.time.load(AtomicOrdering::Relaxed)
        }
    }

    // ---- Timestamp ordering tests (from G1) ----

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

    // ---- HLC::now() monotonicity tests ----

    #[test]
    fn now_returns_monotonically_increasing_timestamps() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        let ts1 = hlc.now();
        let ts2 = hlc.now();
        let ts3 = hlc.now();

        assert!(ts1 < ts2);
        assert!(ts2 < ts3);
    }

    #[test]
    fn now_increments_counter_when_clock_unchanged() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        let ts1 = hlc.now();
        let ts2 = hlc.now();
        let ts3 = hlc.now();

        assert_eq!(ts1.millis, 1_000_000);
        assert_eq!(ts2.millis, 1_000_000);
        assert_eq!(ts3.millis, 1_000_000);

        assert_eq!(ts1.counter, 0);
        assert_eq!(ts2.counter, 1);
        assert_eq!(ts3.counter, 2);
    }

    #[test]
    fn now_resets_counter_when_clock_advances() {
        let (clock, time) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        let ts1 = hlc.now();
        assert_eq!(ts1.millis, 1_000_000);
        assert_eq!(ts1.counter, 0);

        time.store(1_000_001, AtomicOrdering::Relaxed);
        let ts2 = hlc.now();
        assert_eq!(ts2.millis, 1_000_001);
        assert_eq!(ts2.counter, 0);
    }

    #[test]
    fn now_100_calls_all_unique_and_increasing() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        let mut timestamps = Vec::new();
        for _ in 0..100 {
            timestamps.push(hlc.now());
        }

        for (i, ts) in timestamps.iter().enumerate() {
            assert_eq!(ts.millis, 1_000_000);
            assert_eq!(ts.counter, i as u32);
        }

        for i in 1..timestamps.len() {
            assert!(timestamps[i - 1] < timestamps[i]);
        }
    }

    // ---- HLC::update() merge tests ----

    #[test]
    fn update_remote_ahead_fast_forwards() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        let remote = Timestamp {
            millis: 1_000_100,
            counter: 5,
            node_id: "remote-node".to_string(),
        };

        hlc.update(&remote).unwrap();
        let ts = hlc.now();

        // Fast-forwarded to remote millis
        assert_eq!(ts.millis, 1_000_100);
        // counter = remote.counter + 1 from update, then +1 from now = 7
        assert_eq!(ts.counter, 7);
    }

    #[test]
    fn update_same_millis_takes_max_counter() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        hlc.now(); // millis: 1_000_000, counter: 0

        let remote = Timestamp {
            millis: 1_000_000,
            counter: 5,
            node_id: "remote-node".to_string(),
        };

        hlc.update(&remote).unwrap();
        let ts = hlc.now();

        assert_eq!(ts.millis, 1_000_000);
        // After update: max(0, 5) + 1 = 6, then now(): 6 + 1 = 7
        assert_eq!(ts.counter, 7);
    }

    #[test]
    fn update_local_ahead_keeps_local() {
        let (clock, _) = FixedClock::new(1_000_100);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        hlc.now(); // millis: 1_000_100, counter: 0

        let remote = Timestamp {
            millis: 1_000_000,
            counter: 10,
            node_id: "remote-node".to_string(),
        };

        hlc.update(&remote).unwrap();
        let ts = hlc.now();

        // Local millis stays ahead
        assert_eq!(ts.millis, 1_000_100);
    }

    #[test]
    fn update_system_time_ahead_resets_counter() {
        let (clock, _) = FixedClock::new(1_000_200);
        let mut hlc = HLC::new("test-node".to_string(), Box::new(clock));

        let remote = Timestamp {
            millis: 1_000_100,
            counter: 5,
            node_id: "remote-node".to_string(),
        };

        hlc.update(&remote).unwrap();
        let ts = hlc.now();

        // System time ahead: counter reset to 0 by update, then +1 by now()
        assert_eq!(ts.millis, 1_000_200);
        assert_eq!(ts.counter, 1);
    }

    // ---- HLC::compare() tests ----

    #[test]
    fn compare_by_millis() {
        let a = Timestamp { millis: 100, counter: 0, node_id: "A".to_string() };
        let b = Timestamp { millis: 200, counter: 0, node_id: "A".to_string() };

        assert_eq!(HLC::compare(&a, &b), Ordering::Less);
        assert_eq!(HLC::compare(&b, &a), Ordering::Greater);
    }

    #[test]
    fn compare_by_counter() {
        let a = Timestamp { millis: 100, counter: 1, node_id: "A".to_string() };
        let b = Timestamp { millis: 100, counter: 5, node_id: "A".to_string() };

        assert_eq!(HLC::compare(&a, &b), Ordering::Less);
        assert_eq!(HLC::compare(&b, &a), Ordering::Greater);
    }

    #[test]
    fn compare_by_node_id() {
        let a = Timestamp { millis: 100, counter: 0, node_id: "A".to_string() };
        let b = Timestamp { millis: 100, counter: 0, node_id: "B".to_string() };

        assert_eq!(HLC::compare(&a, &b), Ordering::Less);
        assert_eq!(HLC::compare(&b, &a), Ordering::Greater);
    }

    #[test]
    fn compare_equal() {
        let a = Timestamp { millis: 100, counter: 5, node_id: "node1".to_string() };
        let b = Timestamp { millis: 100, counter: 5, node_id: "node1".to_string() };

        assert_eq!(HLC::compare(&a, &b), Ordering::Equal);
    }

    // ---- HLC::to_string() / HLC::parse() round-trip tests ----

    #[test]
    fn to_string_format() {
        let ts = Timestamp { millis: 1_234_567_890, counter: 42, node_id: "my-node".to_string() };
        assert_eq!(HLC::to_string(&ts), "1234567890:42:my-node");
    }

    #[test]
    fn parse_valid() {
        let ts = HLC::parse("1234567890:42:my-node").unwrap();
        assert_eq!(ts.millis, 1_234_567_890);
        assert_eq!(ts.counter, 42);
        assert_eq!(ts.node_id, "my-node");
    }

    #[test]
    fn to_string_parse_roundtrip() {
        let original = Timestamp {
            millis: 9_999_999_999_999,
            counter: 1000,
            node_id: "test-node-123".to_string(),
        };
        let serialized = HLC::to_string(&original);
        let parsed = HLC::parse(&serialized).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_invalid_formats() {
        assert!(HLC::parse("invalid").is_err());
        assert!(HLC::parse("123:456").is_err());
        assert!(HLC::parse("").is_err());
    }

    #[test]
    fn parse_node_id_with_dashes() {
        let ts = Timestamp { millis: 100, counter: 0, node_id: "node-with-dashes".to_string() };
        let serialized = HLC::to_string(&ts);
        assert_eq!(serialized, "100:0:node-with-dashes");
        let parsed = HLC::parse(&serialized).unwrap();
        assert_eq!(parsed.node_id, "node-with-dashes");
    }

    // ---- Drift detection tests ----

    #[test]
    fn strict_mode_rejects_excessive_drift() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::with_options(
            "strict-node".to_string(),
            Box::new(clock),
            true,
            5_000,
        );

        let remote = Timestamp {
            millis: 1_010_000, // 10s ahead, exceeds 5s threshold
            counter: 0,
            node_id: "remote-node".to_string(),
        };

        let result = hlc.update(&remote);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Clock drift detected"));
        assert!(err.contains("10000ms ahead"));
        assert!(err.contains("threshold: 5000ms"));
    }

    #[test]
    fn strict_mode_accepts_within_threshold() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::with_options(
            "strict-node".to_string(),
            Box::new(clock),
            true,
            10_000,
        );

        let remote = Timestamp {
            millis: 1_005_000, // 5s ahead, within 10s threshold
            counter: 0,
            node_id: "remote-node".to_string(),
        };

        assert!(hlc.update(&remote).is_ok());
        let ts = hlc.now();
        assert_eq!(ts.millis, 1_005_000);
    }

    #[test]
    fn strict_mode_default_drift_60s() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::with_options(
            "strict-node".to_string(),
            Box::new(clock),
            true,
            60_000,
        );

        // 50s ahead: within 60s threshold
        let within = Timestamp {
            millis: 1_050_000,
            counter: 0,
            node_id: "remote".to_string(),
        };
        assert!(hlc.update(&within).is_ok());

        // 70s ahead: exceeds 60s threshold (need a fresh HLC for clean test)
        let (clock2, _) = FixedClock::new(1_000_000);
        let mut hlc2 = HLC::with_options(
            "strict-node".to_string(),
            Box::new(clock2),
            true,
            60_000,
        );
        let exceeds = Timestamp {
            millis: 1_070_000,
            counter: 0,
            node_id: "remote".to_string(),
        };
        assert!(hlc2.update(&exceeds).is_err());
    }

    #[test]
    fn non_strict_mode_accepts_drift() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::new("permissive-node".to_string(), Box::new(clock));

        let remote = Timestamp {
            millis: 1_100_000, // 100s ahead
            counter: 0,
            node_id: "remote-node".to_string(),
        };

        // Should NOT error
        assert!(hlc.update(&remote).is_ok());

        // Timestamp accepted
        let ts = hlc.now();
        assert_eq!(ts.millis, 1_100_000);
    }

    #[test]
    fn negative_drift_not_rejected() {
        let (clock, _) = FixedClock::new(1_000_000);
        let mut hlc = HLC::with_options(
            "strict-node".to_string(),
            Box::new(clock),
            true,
            5_000,
        );

        let remote = Timestamp {
            millis: 900_000, // 100s BEHIND
            counter: 0,
            node_id: "remote-node".to_string(),
        };

        // Negative drift should not trigger rejection
        assert!(hlc.update(&remote).is_ok());
    }

    // ---- Accessor tests ----

    #[test]
    fn accessors() {
        let (clock, _) = FixedClock::new(0);
        let hlc = HLC::with_options("node-1".to_string(), Box::new(clock), true, 30_000);

        assert_eq!(hlc.node_id(), "node-1");
        assert!(hlc.strict_mode());
        assert_eq!(hlc.max_drift_ms(), 30_000);
    }

    #[test]
    fn default_accessors() {
        let (clock, _) = FixedClock::new(0);
        let hlc = HLC::new("node-2".to_string(), Box::new(clock));

        assert_eq!(hlc.node_id(), "node-2");
        assert!(!hlc.strict_mode());
        assert_eq!(hlc.max_drift_ms(), 60_000);
    }

    // ---- Multi-node tests ----

    #[test]
    fn total_ordering_across_concurrent_nodes() {
        let (c1, _) = FixedClock::new(1_000_000);
        let (c2, _) = FixedClock::new(1_000_000);
        let (c3, _) = FixedClock::new(1_000_000);

        let mut hlc1 = HLC::new("node-A".to_string(), Box::new(c1));
        let mut hlc2 = HLC::new("node-B".to_string(), Box::new(c2));
        let mut hlc3 = HLC::new("node-C".to_string(), Box::new(c3));

        let ts1 = hlc1.now();
        let ts2 = hlc2.now();
        let ts3 = hlc3.now();

        // Same millis and counter, different node IDs
        assert_eq!(ts1.millis, ts2.millis);
        assert_eq!(ts2.millis, ts3.millis);
        assert_eq!(ts1.counter, ts2.counter);

        // Sort provides total ordering via node_id
        let mut sorted = vec![ts1.clone(), ts2.clone(), ts3.clone()];
        sorted.sort();

        assert_eq!(sorted[0].node_id, "node-A");
        assert_eq!(sorted[1].node_id, "node-B");
        assert_eq!(sorted[2].node_id, "node-C");
    }

    #[test]
    fn clock_sync_between_nodes() {
        let (c1, _) = FixedClock::new(1_000_000);
        let (c2, _) = FixedClock::new(1_000_000);

        let mut hlc1 = HLC::new("node-1".to_string(), Box::new(c1));
        let mut hlc2 = HLC::new("node-2".to_string(), Box::new(c2));

        // Node 1 generates timestamps
        hlc1.now();
        hlc1.now();
        let ts1 = hlc1.now(); // counter: 2

        // Node 2 receives from node 1
        hlc2.update(&ts1).unwrap();
        let ts2 = hlc2.now();

        // Node 2's timestamp must be after node 1's
        assert!(ts1 < ts2);
        assert_eq!(ts2.millis, ts1.millis);
        assert!(ts2.counter > ts1.counter);
    }

    #[test]
    fn bidirectional_communication() {
        let (c1, _) = FixedClock::new(1_000_000);
        let (c2, _) = FixedClock::new(1_000_000);

        let mut hlc1 = HLC::new("node-1".to_string(), Box::new(c1));
        let mut hlc2 = HLC::new("node-2".to_string(), Box::new(c2));

        // Node 1 -> Node 2
        let msg1 = hlc1.now();
        hlc2.update(&msg1).unwrap();

        // Node 2 -> Node 1
        let msg2 = hlc2.now();
        hlc1.update(&msg2).unwrap();

        // Node 1 generates new timestamp
        let final1 = hlc1.now();

        // All strictly ordered
        assert!(msg1 < msg2);
        assert!(msg2 < final1);
    }

    // ---- clock_source() accessor test ----

    #[test]
    fn clock_source_accessor() {
        let (clock, time) = FixedClock::new(42_000);
        let hlc = HLC::new("node".to_string(), Box::new(clock));

        assert_eq!(hlc.clock_source().now(), 42_000);
        time.store(99_000, AtomicOrdering::Relaxed);
        assert_eq!(hlc.clock_source().now(), 99_000);
    }

    // ---- Display impl test ----

    #[test]
    fn timestamp_display() {
        let ts = Timestamp { millis: 100, counter: 5, node_id: "n1".to_string() };
        assert_eq!(format!("{ts}"), "100:5:n1");
    }
}
