//! Behavioral red-on-revert tests for the soak harness OR-Map directional
//! no-loss check.
//!
//! The no-loss logic lives in `benches/soak_harness/or_noloss.rs`. The soak bench
//! is declared `harness = false`, so `cargo test` cannot run `#[test]` functions
//! inside it. To exercise the EXACT bench source (so a regression in the
//! comparator reddens here), this integration test pulls that module in directly
//! via `#[path]` and tests the real functions.
//!
//! Two load-bearing properties:
//!
//! 1. **Directional subset.** `missing_acked_adds` is `acked ⊆ observed`. A
//!    dropped acked add must be reported (RED); a "recovered-more" observed set
//!    (extra identities/keys not in the ledger) must NOT be reported (GREEN).
//!    Reverting the comparator to equality — or flipping the subset direction —
//!    turns the recovered-more case RED.
//!
//! 2. **Value-keyed identity (not tag).** The server re-stamps every OR add's HLC
//!    and regenerates the tag from it, so the client tag is never the persisted
//!    identity; the record VALUE is stored verbatim. `observed_values` therefore
//!    keys the recovered set on value, and the ledger records the value. Reverting
//!    that to a tag-keyed observed set makes every server-re-stamped record's tag
//!    structurally absent from the ledger — i.e. a vacuous 100% false loss — which
//!    `value_identity_survives_server_tag_rewrite` below catches.

#[path = "../benches/soak_harness/or_noloss.rs"]
mod or_noloss;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use or_noloss::{missing_acked_adds, observed_values, render_value, OrLedger};
use topgun_core::hlc::{ORMapRecord, Timestamp};
use topgun_core::messages::ORMapEntry;

/// Build an OR record carrying an explicit integer `value` and `tag`. The soak's
/// persistent keyspace stores a stable unique integer value per slot; the tag is
/// whatever the server re-stamped it to.
fn record_v(tag: &str, value: i64) -> ORMapRecord<rmpv::Value> {
    ORMapRecord {
        value: rmpv::Value::from(value),
        timestamp: Timestamp {
            millis: 1,
            counter: 1,
            node_id: "test".to_string(),
        },
        tag: tag.to_string(),
        ttl_ms: None,
    }
}

/// An OR-Map leaf entry from `(tag, value)` active records and tombstone tags.
fn entry_v(key: &str, active: &[(&str, i64)], tombstones: &[&str]) -> ORMapEntry {
    ORMapEntry {
        key: key.to_string(),
        records: active.iter().map(|(t, v)| record_v(t, *v)).collect(),
        tombstones: tombstones.iter().map(ToString::to_string).collect(),
    }
}

fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(ToString::to_string).collect()
}

// ---------------------------------------------------------------------------
// observed_values: active record values MINUS tombstoned ones.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn observed_values_is_active_minus_tombstones() {
    // Add-only (persistent keyspace shape): all active values observed, keyed on
    // VALUE regardless of what tag the server stamped.
    assert_eq!(
        observed_values(&entry_v("k", &[("srv-tag-a", 10), ("srv-tag-b", 20)], &[])),
        set(&["10", "20"])
    );
    // Mixed: the record whose TAG is tombstoned is suppressed; its value drops.
    assert_eq!(
        observed_values(&entry_v(
            "k",
            &[("ta", 10), ("tb", 20), ("tc", 30)],
            &["tb"]
        )),
        set(&["10", "30"])
    );
    // Fully tombstoned (churn keyspace end-state): observed set is empty.
    assert!(observed_values(&entry_v("k", &[("ta", 10)], &["ta"])).is_empty());
    // Tombstone for a tag with no active record: still empty, no panic.
    assert!(observed_values(&entry_v("k", &[], &["ghost"])).is_empty());
}

// ---------------------------------------------------------------------------
// AC2 — the real defect: value identity survives the server's OR tag rewrite.
//
// The server re-stamps every OR add's HLC and regenerates the tag as
// `{millis}:{counter}:{node_id}`, discarding the client tag. A no-loss check
// keyed on the CLIENT tag therefore reports every acked persist add as lost even
// though the record is durably present — a vacuous 100% false loss (the original
// soak's 136-pair "loss"). Keying on the record VALUE, which the server stores
// verbatim, makes the check honest: the acked value is present, so ZERO loss.
//
// RED-ON-REVERT: reverting `ormap_read_all` to key the observed set on tag (the
// pre-fix behaviour) makes `observed` carry server-stamped tag strings that share
// no element with the value-keyed ledger, so this assertion fails (reports the
// acked values as missing).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn value_identity_survives_server_tag_rewrite() {
    // Ledger keys on the client-chosen stable value per slot.
    let acked: HashMap<String, HashSet<String>> =
        HashMap::from([("ork-persist-0".to_string(), set(&["0", "32", "64"]))]);

    // Server-recovered leaf: the records carry SERVER-REGENERATED tags
    // (`{ms}:{ctr}:node`), nothing like the client `pt-*` tags — but their VALUES
    // are exactly the client-supplied stable values, stored verbatim. There are
    // also EXTRA records (re-adds re-stamped under fresh tags) — recovered-more.
    let leaf = entry_v(
        "ork-persist-0",
        &[
            ("1782824961438:0:topgun-server-node", 0),
            ("1782824962944:1:topgun-server-node", 32),
            ("1782824965074:2:topgun-server-node", 64),
            // recovered-more: a re-add of slot 0 under a fresh server tag.
            ("1782824969094:3:topgun-server-node", 0),
        ],
        &[],
    );
    let observed: HashMap<String, HashSet<String>> =
        HashMap::from([("ork-persist-0".to_string(), observed_values(&leaf))]);

    // The value-keyed observed set contains every acked value — ZERO loss.
    let missing = missing_acked_adds(&acked, &observed);
    assert!(
        missing.is_empty(),
        "value-keyed no-loss must see every acked value despite the server tag \
         rewrite, got {missing:?}"
    );

    // Sanity: a TAG-keyed observed set (the reverted behaviour) shares nothing
    // with the value ledger, so the same comparator would falsely report loss.
    let tag_keyed: HashSet<String> = leaf.records.iter().map(|r| r.tag.clone()).collect();
    let observed_by_tag: HashMap<String, HashSet<String>> =
        HashMap::from([("ork-persist-0".to_string(), tag_keyed)]);
    let false_loss = missing_acked_adds(&acked, &observed_by_tag);
    assert_eq!(
        false_loss.len(),
        3,
        "the reverted tag-keyed check would falsely report every acked value lost"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn render_value_is_stable_decimal_for_integers() {
    assert_eq!(render_value(&rmpv::Value::from(0)), "0");
    assert_eq!(render_value(&rmpv::Value::from(1_000_000_i64)), "1000000");
    assert_eq!(render_value(&rmpv::Value::from(-5_i64)), "-5");
}

// ---------------------------------------------------------------------------
// Directional no-loss comparator, red-on-revert (preserved from SPEC-332).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn dropped_acked_add_is_reported_as_loss() {
    // Ledger required value `10` under key `k`; recovery observed nothing for `k`.
    let acked: HashMap<String, HashSet<String>> = HashMap::from([("k".to_string(), set(&["10"]))]);
    let observed: HashMap<String, HashSet<String>> = HashMap::new();

    let missing = missing_acked_adds(&acked, &observed);
    assert_eq!(missing, vec![("k".to_string(), "10".to_string())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_drop_reports_only_the_missing_value() {
    let acked: HashMap<String, HashSet<String>> =
        HashMap::from([("k".to_string(), set(&["10", "20"]))]);
    // `10` survived; `20` was lost.
    let observed: HashMap<String, HashSet<String>> =
        HashMap::from([("k".to_string(), set(&["10"]))]);

    let missing = missing_acked_adds(&acked, &observed);
    assert_eq!(missing, vec![("k".to_string(), "20".to_string())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovered_more_does_not_redden() {
    // The benign WAL-replay asymmetry: every acked add survived, AND recovery
    // surfaced EXTRA values/keys not in the ledger (the churn keyspace's replayed
    // intermediate records). A directional superset check must treat this as no
    // loss.
    //
    // This is the red-on-revert guard: reverting `missing_acked_adds` to equality
    // (also reporting `observed`'s extras) — or flipping the subset direction —
    // makes this assertion fail.
    let acked: HashMap<String, HashSet<String>> = HashMap::from([
        ("persist-0".to_string(), set(&["10", "20"])),
        ("persist-1".to_string(), set(&["30"])),
    ]);
    let observed: HashMap<String, HashSet<String>> = HashMap::from([
        // Same ledgered values PLUS an extra recovered value on the same key.
        ("persist-0".to_string(), set(&["10", "20", "999"])),
        ("persist-1".to_string(), set(&["30"])),
        // An entirely extra key the ledger never tracked (churn-keyspace replay).
        ("ork-7".to_string(), set(&["7777"])),
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
        HashMap::from([("anything".to_string(), set(&["1"]))]);
    assert!(missing_acked_adds(&acked, &observed).is_empty());
}

// ---------------------------------------------------------------------------
// Ledger: acked-only recording + concurrency safety.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ledger_records_only_what_was_added_and_is_concurrency_safe() {
    let ledger = Arc::new(OrLedger::new());
    assert!(ledger.is_empty());

    // Concurrent churn clients recording acked adds (by stable value) on shared +
    // distinct keys — the persistent-keyspace shape.
    let mut handles = Vec::new();
    for client in 0..8u32 {
        let ledger = Arc::clone(&ledger);
        handles.push(tokio::spawn(async move {
            for slot in 0..16u32 {
                let value = i64::from(client) * 1_000_000 + i64::from(slot);
                ledger.record_add(&format!("persist-{}", slot % 4), &value.to_string());
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let snap = ledger.snapshot();
    assert!(!ledger.is_empty());
    // 4 persistent keys; every (client, slot) value landed exactly once.
    assert_eq!(snap.len(), 4);
    let total_values: usize = snap.values().map(HashSet::len).sum();
    assert_eq!(total_values, 8 * 16);

    // And the directional check passes when observed exactly mirrors the ledger.
    assert!(missing_acked_adds(&snap, &snap).is_empty());
}
