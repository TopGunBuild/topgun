//! Authoritative client-side state model and convergence comparison.
//!
//! The soak uses a **bounded, single-writer-per-key** keyspace: key `k-{i}` is
//! owned exclusively by churn client `i % clients`. Single-writer ownership
//! makes the expected value of every key unambiguous (no concurrent-writer
//! race), which is what lets the convergence check be exact rather than
//! best-effort.
//!
//! Two properties fall out of this design:
//!
//! 1. **Convergence is decidable.** After a quiesced checkpoint the server's
//!    read-back must equal this model for every touched key. Any mismatch is a
//!    real divergence — the heart of the soak.
//! 2. **Memory is bounded by construction.** A fixed keyspace overwritten in
//!    place means legitimate data does not grow, so a rising RSS implicates a
//!    leak (e.g. unbounded OR-Map tombstone accumulation, TODO-479/480) rather
//!    than legitimate dataset growth.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

/// Process-wide logical counter feeding the HLC `counter` field. Strictly
/// increasing, so successive writes (including post-reconnect resends) always
/// win Last-Write-Wins against any earlier value for the same key.
static LOGICAL_CLOCK: AtomicU64 = AtomicU64::new(1);

/// Allocate the next monotonic `(millis, counter)` HLC pair for a write.
#[allow(clippy::cast_possible_truncation)]
pub fn next_stamp() -> (u64, u32) {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let counter = LOGICAL_CLOCK.fetch_add(1, Ordering::Relaxed) as u32;
    (millis, counter)
}

/// The harness's authoritative view of what every key's value should be: the
/// latest value it has successfully acked. Shared across all churn clients and
/// the convergence verifier.
pub struct Model {
    /// `key -> latest acked value`.
    values: DashMap<String, i64>,
    keyspace: usize,
    clients: usize,
}

impl Model {
    pub fn new(keyspace: usize, clients: usize) -> Self {
        Self {
            values: DashMap::new(),
            keyspace,
            clients: clients.max(1),
        }
    }

    /// Key name for slot `i`.
    pub fn key_for(i: usize) -> String {
        format!("k-{i}")
    }

    /// The slots owned (solely written) by churn client `client_idx`.
    pub fn keys_owned_by(&self, client_idx: usize) -> Vec<usize> {
        (0..self.keyspace)
            .filter(|i| i % self.clients == client_idx % self.clients)
            .collect()
    }

    /// Record an acked write so the model reflects the new expected value.
    pub fn record(&self, key: &str, value: i64) {
        self.values.insert(key.to_string(), value);
    }

    /// Snapshot the full expected state. Taken while churn is quiesced, so it is
    /// a consistent point-in-time view.
    pub fn snapshot(&self) -> HashMap<String, i64> {
        self.values
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect()
    }
}

/// One disagreement between the model and the server's read-back.
#[derive(Debug, Clone)]
pub struct Divergence {
    pub key: String,
    pub expected: Option<i64>,
    pub actual: Option<i64>,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "key={} expected={:?} actual={:?}",
            self.key, self.expected, self.actual
        )
    }
}

/// Compare the harness model against a server read-back. Every key the harness
/// has written must be present on the server with the identical value, and the
/// server must hold no key the harness never wrote. Returns all disagreements.
pub fn compare(expected: &HashMap<String, i64>, actual: &HashMap<String, i64>) -> Vec<Divergence> {
    let mut diffs = Vec::new();
    for (key, exp) in expected {
        match actual.get(key) {
            Some(act) if act == exp => {}
            other => diffs.push(Divergence {
                key: key.clone(),
                expected: Some(*exp),
                actual: other.copied(),
            }),
        }
    }
    for (key, act) in actual {
        if !expected.contains_key(key) {
            diffs.push(Divergence {
                key: key.clone(),
                expected: None,
                actual: Some(*act),
            });
        }
    }
    diffs.sort_by(|a, b| a.key.cmp(&b.key));
    diffs
}
