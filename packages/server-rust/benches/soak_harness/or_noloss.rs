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

    /// Point-in-time snapshot of the acked-add set per key. Taken while churn is
    /// quiesced at a checkpoint, so it is a consistent view.
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

/// The observed tag set for one OR-Map sync leaf entry: active record tags MINUS
/// tombstone tags. This is the user-visible OR-Map content for the key — a tag
/// that has been tombstoned is no longer observed.
pub fn observed_tags(entry: &ORMapEntry) -> HashSet<String> {
    let tombstones: HashSet<&str> = entry.tombstones.iter().map(String::as_str).collect();
    entry
        .records
        .iter()
        .map(|r| r.tag.as_str())
        .filter(|tag| !tombstones.contains(tag))
        .map(ToString::to_string)
        .collect()
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
