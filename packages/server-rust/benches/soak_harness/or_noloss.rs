//! OR-Map crash-recovery directional no-loss check.
//!
//! The soak proves LWW "acked == durable" across a `kill -9`. This module adds
//! the OR-Map analogue: an acked OR-Map add must never be lost across crash
//! recovery, while the benign WAL-replay "recovered-more" asymmetry must never
//! produce a false failure.
//!
//! ## Why a directional superset, not equality
//!
//! The OR-Map merkle root legitimately DIVERGES pre-kill vs post-recovery under
//! the WAL-only window: the live in-memory tree net-compacts/GCs an
//! add-then-remove pair, while WAL replay reconstructs the intermediate add tag
//! and its tombstone. So post-recovery can hold MORE tags/tombstones than the
//! live tree ("recovered-more") even though no observed value was lost. A root
//! equality check would false-red on that. The honest invariant is directional:
//!
//! > every acked OR add must still be observed after recovery; extra recovered
//! > tags are benign.
//!
//! ## Why an add-only persistent keyspace
//!
//! Tracking removes in the ledger opens a kill-window false-loss race: an add is
//! acked (in the ledger) and its remove is durably applied (tombstone fsynced)
//! but the remove-ack never reaches the client before the `kill -9`. The ledger
//! still holds the tag; the post-recovery observed set (active minus tombstones)
//! correctly lacks it — a FALSE loss. The fix is to seed the ledger from a
//! separate keyspace that is ONLY ever added to, never removed. With no
//! tombstones in the persistent keyspace, WAL replay of its ops can only add
//! active tags, never suppress one, so the post-recovery observed set is provably
//! a strict superset of the ledger. The remove-driven tombstone-growth churn
//! lives in a different keyspace that is excluded from the loss check.

use std::collections::{HashMap, HashSet};

use dashmap::DashMap;
use topgun_core::messages::ORMapEntry;

/// Thread-safe ledger of acked **persistent** OR-Map adds: persistent key → set
/// of acked add tags that MUST survive crash recovery.
///
/// Updated only on an `or_add` ACK (an add whose ack never returned is never
/// inserted, so a genuine kill-window drop of an *unacked* add is not miscounted
/// as loss). The persistent keyspace is never removed from, so the ledger needs
/// no tombstone reconciliation.
#[derive(Default)]
pub struct OrLedger {
    adds: DashMap<String, HashSet<String>>,
}

impl OrLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an acked persistent OR add. Safe under concurrent churn clients
    /// (`DashMap` entry holds a per-shard write lock across the insert).
    pub fn record_add(&self, key: &str, tag: &str) {
        self.adds
            .entry(key.to_string())
            .or_default()
            .insert(tag.to_string());
    }

    /// Snapshot of the acked-add set per key. `DashMap::iter` locks shards one at
    /// a time rather than globally, so this is not a single atomic instant; that
    /// is fine because the caller snapshots while churn is paused AND before the
    /// post-recovery read, so any concurrent `record_add` is either already
    /// applied on the server before that read or simply absent from the snapshot —
    /// never a tag that is in `acked` but legitimately not yet on the server.
    pub fn snapshot(&self) -> HashMap<String, HashSet<String>> {
        self.adds
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// True when no persistent add has been acked yet.
    pub fn is_empty(&self) -> bool {
        self.adds.is_empty()
    }
}

/// The observed **value** set for one OR-Map sync leaf entry: the rendered
/// `value` of every non-tombstoned active record.
///
/// ## Why the no-loss ledger must key on value, not tag
///
/// The server re-stamps every OR add's HLC (security: a forged client HLC must
/// not win a future conflict) and, because the OR tag is derived from that HLC
/// (`{millis}:{counter}:{node_id}`), it regenerates the tag as well. So the tag
/// the *client* chose for an add is never the tag the server persists — keying a
/// directional no-loss check on the client tag makes it vacuously red (the
/// client tag is structurally absent from every recovered record). The record
/// `value`, by contrast, is client-supplied and the server stores it verbatim,
/// so it is the identity that actually survives sanitization and crash recovery.
/// The persistent keyspace therefore writes a stable unique value per owned slot
/// and the ledger reconciles on that value.
///
/// A tombstoned tag's value is excluded — a removed record is no longer observed.
///
/// ## Coverage limit: per-value, not per-add
///
/// The server dedups OR records by tag and stamps a fresh tag on every add, so N
/// re-adds of the same `(key, value)` are stored as N distinct records. This set
/// collapses them to one element, and the ledger likewise records the value once.
/// The check therefore verifies that each acked VALUE survives recovery, not that
/// each individual acked ADD does: losing M of N re-adds of the same value while
/// at least one survives is not detected. The persistent keyspace writes one stable
/// unique value per owned slot, so per-slot durability IS covered; closing the
/// per-add gap needs the server-regenerated tag surfaced to the client (tracked in
/// TODO-558), which is out of scope here.
pub fn observed_values(entry: &ORMapEntry) -> HashSet<String> {
    let tombstones: HashSet<&str> = entry.tombstones.iter().map(String::as_str).collect();
    entry
        .records
        .iter()
        .filter(|r| !tombstones.contains(r.tag.as_str()))
        .map(|r| render_value(&r.value))
        .collect()
}

/// Render an OR record `value` to the stable string identity the ledger keys on.
/// Integer values (the soak's persistent-keyspace shape) render to their decimal
/// form; any other shape falls back to its debug rendering so the check still has
/// a deterministic, comparable identity.
///
/// Both `as_i64` and `as_u64` are tried so a positive integer that msgpack
/// round-trips as an unsigned variant still renders to the same decimal string the
/// ledger recorded — otherwise the debug fallback (`UInt(42)` vs `42`) would make
/// the check falsely RED (fail-closed, but flaky).
pub fn render_value(value: &rmpv::Value) -> String {
    if let Some(n) = value.as_i64() {
        return n.to_string();
    }
    if let Some(n) = value.as_u64() {
        return n.to_string();
    }
    format!("{value:?}")
}

/// Directional OR-Map no-loss diff: every acked (net-present) OR add tag must
/// appear in the post-recovery observed set.
///
/// Returns the `(key, tag)` pairs that are MISSING from `observed` — each one a
/// HARD loss of an acked add. An empty result means no loss.
///
/// The comparison is **`acked ⊆ observed`** and ONLY that direction.
/// "Recovered-more" — `observed` holding extra keys or tags not present in
/// `acked` — is benign and is NEVER reported. Flipping this to equality (also
/// reporting `observed`'s extras) or to the reverse subset would re-introduce the
/// false-positive "recovered-more" failure this check exists to avoid, and could
/// mask a real loss. This direction is load-bearing.
pub fn missing_acked_adds(
    acked: &HashMap<String, HashSet<String>>,
    observed: &HashMap<String, HashSet<String>>,
) -> Vec<(String, String)> {
    let mut missing = Vec::new();
    for (key, tags) in acked {
        let obs = observed.get(key);
        for tag in tags {
            let present = obs.is_some_and(|set| set.contains(tag));
            if !present {
                missing.push((key.clone(), tag.clone()));
            }
        }
    }
    missing.sort();
    missing
}
