//! Write-Ahead Log (WAL) trait surface and supporting types.
//!
//! Defines the frozen contract that both the production WAL writer (307c) and
//! the in-memory simulation double (307d) implement. Having the trait defined
//! independently of any file I/O keeps 307c and 307d from being coupled to
//! one another's implementation details.
//!
//! The WAL is per-partition: one append-only file per partition under a
//! configured `wal_dir`. The `Wal` trait is layout-agnostic — it operates on
//! individual `WalEntry` records and lets the writer own the file-per-partition
//! mapping.

pub mod format;

use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

// ---------------------------------------------------------------------------
// WalFsyncPolicy
// ---------------------------------------------------------------------------

/// Controls how aggressively the WAL writer calls `fsync` after writing frames.
///
/// Choosing the right policy is a crash-safety vs. throughput tradeoff:
/// - `PerOp` maximises durability at the cost of per-write syscall overhead.
/// - `Batched` amortises fsync cost across a group of writes (the default).
/// - `None` skips fsync entirely — useful for tests and throughput benchmarks
///   where crash-safety is not required.
///
/// The *behaviour* of the policy (actually calling fsync) is implemented in the
/// `WalWriter` (307c). This enum is the configuration carrier only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WalFsyncPolicy {
    /// Call `fsync` after every appended frame. Safest; highest latency.
    PerOp,
    /// Call `fsync` after each flush batch. Default — good balance of safety
    /// and throughput.
    #[default]
    Batched,
    /// Never call `fsync`. OS-buffered writes only. Not crash-safe.
    None,
}

/// Parsing error returned when an unknown policy string is encountered.
///
/// The env-parse contract is: unknown values are rejected rather than silently
/// defaulted, so a misconfigured deployment surfaces an error at startup instead
/// of quietly running with weaker durability guarantees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWalFsyncPolicyError(pub String);

impl std::fmt::Display for ParseWalFsyncPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown WAL fsync policy {:?}; valid values are per_op, batched, none",
            self.0
        )
    }
}

impl std::error::Error for ParseWalFsyncPolicyError {}

impl FromStr for WalFsyncPolicy {
    type Err = ParseWalFsyncPolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "per_op" => Ok(Self::PerOp),
            "batched" => Ok(Self::Batched),
            "none" => Ok(Self::None),
            other => Err(ParseWalFsyncPolicyError(other.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// WalOp — op enum mirroring DelayedOp
// ---------------------------------------------------------------------------

/// The operation recorded in a WAL entry, mirroring the write-behind `DelayedOp`
/// shape so that WAL-driven recovery can replay the exact same operation.
///
/// Using a typed enum rather than a string tag avoids the silent type-mismatch
/// class of bugs and lets serde enforce the variant structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WalOp {
    /// Upsert: the record value and its TTL expiration timestamp in milliseconds.
    Store {
        /// The full CRDT value to persist.
        value: Value,
        /// Wall-clock expiration time in milliseconds since epoch.
        /// Negative or zero means no expiration.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        expiration_time: Option<i64>,
    },
    /// Tombstone: remove the record from the store.
    Remove,
}

// ---------------------------------------------------------------------------
// WalEntry
// ---------------------------------------------------------------------------

/// A single record in the Write-Ahead Log.
///
/// Each entry captures the map, key, operation, HLC timestamp (for idempotent
/// replay deduplication), and a monotonic sequence number for ordering.
///
/// The HLC `Timestamp` is copied directly from `RecordValue::Lww { timestamp }`
/// so that recovery can skip entries that have already been superseded by a
/// higher-timestamped write — no new field on `RecordValue` is needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalEntry {
    /// Map name this entry belongs to.
    pub map: String,
    /// Record key within the map.
    pub key: String,
    /// The operation to replay on recovery.
    pub op: WalOp,
    /// HLC timestamp from the originating write. Used as the idempotency key:
    /// during recovery, a replayed entry whose timestamp is older than the
    /// current in-memory value is a no-op.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timestamp: Option<Timestamp>,
    /// Monotonically increasing counter assigned at append time. Establishes
    /// total ordering within a partition WAL file for ordered replay.
    pub sequence: u64,
}

// ---------------------------------------------------------------------------
// Wal trait
// ---------------------------------------------------------------------------

/// Object-safe WAL interface.
///
/// Both the production file-backed `WalWriter` (307c) and the in-memory
/// simulation double (307d) implement this trait so callers can depend on
/// `Arc<dyn Wal>` without coupling to a specific implementation.
///
/// All methods are async because the production implementation performs I/O.
/// The fsync behaviour is governed by the `WalFsyncPolicy` that each
/// implementation is configured with — callers do not call fsync directly.
#[async_trait]
pub trait Wal: Send + Sync {
    /// Appends a `WalEntry` to the log for the given partition.
    ///
    /// The implementation is responsible for encoding the entry, writing it to
    /// the appropriate per-partition file, and fsyncing according to the
    /// configured `WalFsyncPolicy`. Returns an error if the write or fsync
    /// fails; the caller should NOT ack the client until this returns `Ok`.
    async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()>;

    /// Marks a WAL entry as applied and removes it from the set of entries
    /// that `unapplied` would return.
    ///
    /// Called after the entry has been successfully flushed to the durable
    /// inner store. The implementation may truncate or checkpoint the log
    /// when all entries up to a sequence number have been marked applied.
    async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()>;

    /// Returns all un-applied entries for a partition, in sequence order.
    ///
    /// Called at startup (307c recovery loop) to replay any entries that were
    /// appended before the last crash but not yet acknowledged as applied.
    /// An empty vec means the partition is clean.
    async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>>;
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // -----------------------------------------------------------------------
    // WalFsyncPolicy from-str tests
    // -----------------------------------------------------------------------

    #[test]
    fn fsync_policy_parse_per_op() {
        assert_eq!(
            "per_op".parse::<WalFsyncPolicy>().unwrap(),
            WalFsyncPolicy::PerOp
        );
    }

    #[test]
    fn fsync_policy_parse_batched() {
        assert_eq!(
            "batched".parse::<WalFsyncPolicy>().unwrap(),
            WalFsyncPolicy::Batched
        );
    }

    #[test]
    fn fsync_policy_parse_none() {
        assert_eq!(
            "none".parse::<WalFsyncPolicy>().unwrap(),
            WalFsyncPolicy::None
        );
    }

    #[test]
    fn fsync_policy_parse_unknown_is_rejected() {
        // Unknown values must be rejected so a misconfigured deployment surfaces
        // an error at startup rather than silently using the wrong durability level.
        let result = "always".parse::<WalFsyncPolicy>();
        assert!(
            result.is_err(),
            "Unknown policy string should return Err, not a default"
        );
        let err = result.unwrap_err();
        // The error message must name the bad value for operator debuggability.
        assert!(
            err.to_string().contains("always"),
            "Error should name the bad value"
        );
    }

    #[test]
    fn fsync_policy_default_is_batched() {
        assert_eq!(WalFsyncPolicy::default(), WalFsyncPolicy::Batched);
    }

    // -----------------------------------------------------------------------
    // Arc<dyn Wal> object-safety test
    //
    // Demonstrates that `Wal` is object-safe and can be held behind `Arc<dyn
    // Wal>`. This is the injection mechanism used by 307c (production writer)
    // and 307d (sim double).
    // -----------------------------------------------------------------------

    /// Minimal in-memory WAL used to prove object-safety.
    struct InMemoryWal {
        entries: tokio::sync::Mutex<Vec<(u32, WalEntry)>>,
        applied: tokio::sync::Mutex<Vec<(u32, u64)>>,
    }

    impl InMemoryWal {
        fn new() -> Self {
            Self {
                entries: tokio::sync::Mutex::new(Vec::new()),
                applied: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Wal for InMemoryWal {
        async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
            self.entries.lock().await.push((partition, entry.clone()));
            Ok(())
        }

        async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
            self.applied.lock().await.push((partition, sequence));
            Ok(())
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            let guard = self.entries.lock().await;
            let applied_guard = self.applied.lock().await;
            let applied_seqs: std::collections::HashSet<u64> = applied_guard
                .iter()
                .filter(|(p, _)| *p == partition)
                .map(|(_, seq)| *seq)
                .collect();
            Ok(guard
                .iter()
                .filter(|(p, e)| *p == partition && !applied_seqs.contains(&e.sequence))
                .map(|(_, e)| e.clone())
                .collect())
        }
    }

    #[tokio::test]
    async fn wal_trait_is_object_safe() {
        // Constructing an Arc<dyn Wal> verifies the trait is object-safe.
        let wal: Arc<dyn Wal> = Arc::new(InMemoryWal::new());

        let entry = WalEntry {
            map: "map1".to_string(),
            key: "key1".to_string(),
            op: WalOp::Remove,
            timestamp: None,
            sequence: 1,
        };

        wal.append(0, &entry).await.unwrap();

        let unapplied = wal.unapplied(0).await.unwrap();
        assert_eq!(unapplied.len(), 1);

        wal.mark_applied(0, 1).await.unwrap();

        let unapplied_after = wal.unapplied(0).await.unwrap();
        assert!(
            unapplied_after.is_empty(),
            "No entries should remain after mark_applied"
        );
    }
}
