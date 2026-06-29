//! Behavioral red-on-revert tests for the soak harness OR-Map directional
//! no-loss check.
//!
//! The no-loss logic lives in `benches/soak_harness/or_noloss.rs`. The soak bench
//! is declared `harness = false`, so `cargo test` cannot run `#[test]` functions
//! inside it. To exercise the EXACT bench source (so a regression in the
//! comparator reddens here), this integration test pulls that module in directly
//! via `#[path]` and tests the real functions.
//!
//! The load-bearing property: `missing_acked_adds` is a directional subset check
//! (`acked ⊆ observed`). A dropped acked add must be reported (RED); a
//! "recovered-more" observed set (extra tags/keys/tombstones not in the ledger)
//! must NOT be reported (GREEN). Reverting the comparator to equality — or
//! flipping the subset direction — turns the recovered-more case RED, which is
//! precisely the regression these tests guard.

#[path = "../benches/soak_harness/or_noloss.rs"]
mod or_noloss;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use or_noloss::{missing_acked_adds, observed_tags, OrLedger};
use topgun_core::hlc::{ORMapRecord, Timestamp};
use topgun_core::messages::ORMapEntry;

fn record(tag: &str) -> ORMapRecord<rmpv::Value> {
    ORMapRecord {
        value: rmpv::Value::from(1),
        timestamp: Timestamp {
            millis: 1,
            counter: 1,
            node_id: "test".to_string(),
        },
        tag: tag.to_string(),
        ttl_ms: None,
    }
}

fn entry(key: &str, active: &[&str], tombstones: &[&str]) -> ORMapEntry {
    ORMapEntry {
        key: key.to_string(),
        records: active.iter().map(|t| record(t)).collect(),
        tombstones: tombstones.iter().map(ToString::to_string).collect(),
    }
}

fn set(tags: &[&str]) -> HashSet<String> {
    tags.iter().map(ToString::to_string).collect()
}

// ---------------------------------------------------------------------------
// AC2 — observed-set parse: active record tags MINUS tombstone tags.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn observed_tags_is_active_minus_tombstones() {
    // Add-only (persistent keyspace shape): all active tags observed.
    assert_eq!(
        observed_tags(&entry("k", &["a", "b"], &[])),
        set(&["a", "b"])
    );
    // Mixed: a tombstoned tag is suppressed; the rest remain observed.
    assert_eq!(
        observed_tags(&entry("k", &["a", "b", "c"], &["b"])),
        set(&["a", "c"])
    );
    // Fully tombstoned (churn keyspace end-state): observed set is empty.
    assert!(observed_tags(&entry("k", &["a"], &["a"])).is_empty());
    // Tombstone for a tag with no active record: still empty, no panic.
    assert!(observed_tags(&entry("k", &[], &["ghost"])).is_empty());
}

// ---------------------------------------------------------------------------
// AC3 — directional no-loss comparator, red-on-revert.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn dropped_acked_add_is_reported_as_loss() {
    // Ledger required tag `t1` under key `k`; recovery observed nothing for `k`.
    let acked: HashMap<String, HashSet<String>> = HashMap::from([("k".to_string(), set(&["t1"]))]);
    let observed: HashMap<String, HashSet<String>> = HashMap::new();

    let missing = missing_acked_adds(&acked, &observed);
    assert_eq!(missing, vec![("k".to_string(), "t1".to_string())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_drop_reports_only_the_missing_tag() {
    let acked: HashMap<String, HashSet<String>> =
        HashMap::from([("k".to_string(), set(&["t1", "t2"]))]);
    // `t1` survived; `t2` was lost.
    let observed: HashMap<String, HashSet<String>> =
        HashMap::from([("k".to_string(), set(&["t1"]))]);

    let missing = missing_acked_adds(&acked, &observed);
    assert_eq!(missing, vec![("k".to_string(), "t2".to_string())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovered_more_does_not_redden() {
    // The benign WAL-replay asymmetry: every acked add survived, AND recovery
    // surfaced EXTRA tags/keys not in the ledger (the churn keyspace's replayed
    // intermediate tags). A directional superset check must treat this as no loss.
    //
    // This is the red-on-revert guard: reverting `missing_acked_adds` to equality
    // (also reporting `observed`'s extras) — or flipping the subset direction —
    // makes this assertion fail.
    let acked: HashMap<String, HashSet<String>> = HashMap::from([
        ("persist-0".to_string(), set(&["pa", "pb"])),
        ("persist-1".to_string(), set(&["pc"])),
    ]);
    let observed: HashMap<String, HashSet<String>> = HashMap::from([
        // Same ledgered tags PLUS an extra recovered tag on the same key.
        (
            "persist-0".to_string(),
            set(&["pa", "pb", "extra-replayed"]),
        ),
        ("persist-1".to_string(), set(&["pc"])),
        // An entirely extra key the ledger never tracked (churn-keyspace replay).
        ("ork-7".to_string(), set(&["churn-tag"])),
    ]);

    let missing = missing_acked_adds(&acked, &observed);
    assert!(
        missing.is_empty(),
        "recovered-more must never redden, got {missing:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_ledger_never_reports_loss() {
    let acked: HashMap<String, HashSet<String>> = HashMap::new();
    let observed: HashMap<String, HashSet<String>> =
        HashMap::from([("anything".to_string(), set(&["x"]))]);
    assert!(missing_acked_adds(&acked, &observed).is_empty());
}

// ---------------------------------------------------------------------------
// Ledger: acked-only recording + concurrency safety.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ledger_records_only_what_was_added_and_is_concurrency_safe() {
    let ledger = Arc::new(OrLedger::new());
    assert!(ledger.is_empty());

    // Concurrent churn clients recording acked adds on shared + distinct keys.
    let mut handles = Vec::new();
    for client in 0..8u32 {
        let ledger = Arc::clone(&ledger);
        handles.push(tokio::spawn(async move {
            for slot in 0..16u32 {
                // Shared key across clients (different tags), distinct tag per
                // (client, slot) — the persistent-keyspace shape.
                ledger.record_add(
                    &format!("persist-{}", slot % 4),
                    &format!("pt-{client}-{slot}"),
                );
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let snap = ledger.snapshot();
    assert!(!ledger.is_empty());
    // 4 persistent keys; every (client, slot) tag landed exactly once.
    assert_eq!(snap.len(), 4);
    let total_tags: usize = snap.values().map(HashSet::len).sum();
    assert_eq!(total_tags, 8 * 16);

    // And the directional check passes when observed exactly mirrors the ledger.
    assert!(missing_acked_adds(&snap, &snap).is_empty());
}
