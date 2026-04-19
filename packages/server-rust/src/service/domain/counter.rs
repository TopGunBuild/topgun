//! Counter registry for PN-Counter CRDT operations.
//!
//! Maintains an in-memory map of named PN-Counters with subscriber tracking.
//! Each counter stores `PNCounterState` (per-node positive/negative values)
//! and a set of `ConnectionId`s that are subscribed to updates.

use std::collections::HashMap;

use dashmap::{DashMap, DashSet};

use topgun_core::messages::PNCounterState;

use crate::network::connection::ConnectionId;

// ---------------------------------------------------------------------------
// CounterRegistry
// ---------------------------------------------------------------------------

/// Thread-safe in-memory PN-Counter store with subscriber tracking.
///
/// Uses `DashMap` for concurrent access, consistent with the `TopicRegistry`
/// pattern used in the messaging domain service.
pub struct CounterRegistry {
    /// Per-counter merged state.
    counters: DashMap<String, PNCounterState>,
    /// Per-counter subscriber set.
    subscribers: DashMap<String, DashSet<ConnectionId>>,
    /// Node ID for this server instance (used for local increments if needed).
    #[allow(dead_code)]
    node_id: String,
}

impl CounterRegistry {
    /// Creates a new empty counter registry.
    #[must_use]
    pub fn new(node_id: String) -> Self {
        Self {
            counters: DashMap::new(),
            subscribers: DashMap::new(),
            node_id,
        }
    }

    /// Returns the current state of the named counter, creating an empty
    /// counter if it does not exist.
    #[must_use]
    pub fn get_or_create(&self, name: &str) -> PNCounterState {
        self.counters
            .entry(name.to_string())
            .or_insert_with(|| PNCounterState {
                p: HashMap::new(),
                n: HashMap::new(),
            })
            .clone()
    }

    /// Merges incoming `PNCounterState` into the local state for the named
    /// counter using max-per-node semantics.
    ///
    /// For each node ID in `incoming.p`, the result is
    /// `max(local.p[node], incoming.p[node])`. Same for `n`.
    ///
    /// Returns the merged state.
    #[must_use]
    pub fn merge(&self, name: &str, incoming: &PNCounterState) -> PNCounterState {
        let mut entry = self
            .counters
            .entry(name.to_string())
            .or_insert_with(|| PNCounterState {
                p: HashMap::new(),
                n: HashMap::new(),
            });

        // Merge positive counters
        for (node, &value) in &incoming.p {
            let local_val = entry.p.entry(node.clone()).or_insert(0.0);
            if value > *local_val {
                *local_val = value;
            }
        }

        // Merge negative counters
        for (node, &value) in &incoming.n {
            let local_val = entry.n.entry(node.clone()).or_insert(0.0);
            if value > *local_val {
                *local_val = value;
            }
        }

        entry.clone()
    }

    /// Adds a subscriber to the named counter.
    pub fn subscribe(&self, name: &str, conn_id: ConnectionId) {
        self.subscribers
            .entry(name.to_string())
            .or_default()
            .insert(conn_id);
    }

    /// Removes a subscriber from the named counter.
    pub fn unsubscribe(&self, name: &str, conn_id: ConnectionId) {
        if let Some(subs) = self.subscribers.get(name) {
            subs.remove(&conn_id);
            if subs.is_empty() {
                drop(subs);
                // Remove the entry if no subscribers remain.
                // Re-check after acquiring write lock to avoid race.
                self.subscribers.remove_if(name, |_, v| v.is_empty());
            }
        }
    }

    /// Removes a connection from all counter subscriber sets.
    ///
    /// Called on connection disconnect to clean up subscriptions.
    pub fn unsubscribe_all(&self, conn_id: ConnectionId) {
        let mut empty_keys = Vec::new();
        for entry in &self.subscribers {
            entry.value().remove(&conn_id);
            if entry.value().is_empty() {
                empty_keys.push(entry.key().clone());
            }
        }
        for key in empty_keys {
            self.subscribers.remove_if(&key, |_, v| v.is_empty());
        }
    }

    /// Returns all subscribers for the named counter.
    #[must_use]
    pub fn subscribers(&self, name: &str) -> Vec<ConnectionId> {
        self.subscribers
            .get(name)
            .map(|subs| subs.iter().map(|id| *id).collect())
            .unwrap_or_default()
    }

    /// Canonical disconnect-hook entry point.
    ///
    /// Removes `conn_id` from all counter subscriber sets. Thin alias over
    /// `unsubscribe_all` so the three session-scoped registries share a uniform
    /// `release_on_disconnect` surface used by `handle_socket`'s cleanup path.
    pub fn release_on_disconnect(&self, conn_id: ConnectionId) {
        self.unsubscribe_all(conn_id);
    }

    /// Returns the current computed value of the named counter.
    ///
    /// Value = sum(p.values()) - sum(n.values()).
    /// Returns 0.0 if the counter does not exist.
    #[must_use]
    pub fn counter_value(&self, name: &str) -> f64 {
        self.counters.get(name).map_or(0.0, |state| {
            let pos: f64 = state.p.values().sum();
            let neg: f64 = state.n.values().sum();
            pos - neg
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(p: Vec<(&str, f64)>, n: Vec<(&str, f64)>) -> PNCounterState {
        PNCounterState {
            p: p.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            n: n.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }

    fn registry() -> CounterRegistry {
        CounterRegistry::new("test-node".to_string())
    }

    // --- get_or_create ---

    #[test]
    fn get_or_create_returns_empty_state_for_new_counter() {
        let reg = registry();
        let state = reg.get_or_create("hits");
        assert!(state.p.is_empty());
        assert!(state.n.is_empty());
    }

    #[test]
    fn get_or_create_returns_existing_state() {
        let reg = registry();
        let incoming = make_state(vec![("a", 5.0)], vec![]);
        let _ = reg.merge("hits", &incoming);

        let state = reg.get_or_create("hits");
        assert_eq!(state.p.get("a"), Some(&5.0));
    }

    // --- merge semantics ---

    #[test]
    fn merge_takes_max_per_node_for_positive() {
        let reg = registry();
        let s1 = make_state(vec![("a", 3.0), ("b", 7.0)], vec![]);
        let s2 = make_state(vec![("a", 5.0), ("b", 2.0)], vec![]);

        let _ = reg.merge("c1", &s1);
        let merged = reg.merge("c1", &s2);

        assert_eq!(merged.p.get("a"), Some(&5.0)); // max(3, 5)
        assert_eq!(merged.p.get("b"), Some(&7.0)); // max(7, 2)
    }

    #[test]
    fn merge_takes_max_per_node_for_negative() {
        let reg = registry();
        let s1 = make_state(vec![], vec![("x", 4.0)]);
        let s2 = make_state(vec![], vec![("x", 6.0)]);

        let _ = reg.merge("c1", &s1);
        let merged = reg.merge("c1", &s2);

        assert_eq!(merged.n.get("x"), Some(&6.0));
    }

    #[test]
    fn merge_is_commutative() {
        let reg_ab = registry();
        let reg_ba = registry();

        let a = make_state(vec![("n1", 10.0)], vec![("n1", 3.0)]);
        let b = make_state(vec![("n1", 7.0), ("n2", 4.0)], vec![("n1", 5.0)]);

        // A then B
        let _ = reg_ab.merge("c", &a);
        let ab = reg_ab.merge("c", &b);

        // B then A
        let _ = reg_ba.merge("c", &b);
        let ba = reg_ba.merge("c", &a);

        assert_eq!(ab, ba);
    }

    #[test]
    fn merge_is_idempotent() {
        let reg = registry();
        let state = make_state(vec![("a", 3.0)], vec![("b", 2.0)]);

        let first = reg.merge("c", &state);
        let second = reg.merge("c", &state);

        assert_eq!(first, second);
    }

    #[test]
    fn merge_with_empty_incoming_is_identity() {
        let reg = registry();
        let existing = make_state(vec![("a", 5.0)], vec![("b", 1.0)]);
        let empty = make_state(vec![], vec![]);

        let after_existing = reg.merge("c", &existing);
        let after_empty = reg.merge("c", &empty);

        assert_eq!(after_existing, after_empty);
    }

    // --- counter_value ---

    #[test]
    fn counter_value_is_sum_p_minus_sum_n() {
        let reg = registry();
        let state = make_state(vec![("a", 10.0), ("b", 5.0)], vec![("a", 3.0), ("b", 2.0)]);
        let _ = reg.merge("c", &state);

        // value = (10 + 5) - (3 + 2) = 10
        assert!((reg.counter_value("c") - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn counter_value_returns_zero_for_nonexistent() {
        let reg = registry();
        assert!((reg.counter_value("nope") - 0.0).abs() < f64::EPSILON);
    }

    // --- subscribe / unsubscribe ---

    #[test]
    fn subscribe_adds_connection() {
        let reg = registry();
        reg.subscribe("c1", ConnectionId(1));
        reg.subscribe("c1", ConnectionId(2));

        let subs = reg.subscribers("c1");
        assert_eq!(subs.len(), 2);
        assert!(subs.contains(&ConnectionId(1)));
        assert!(subs.contains(&ConnectionId(2)));
    }

    #[test]
    fn unsubscribe_removes_connection() {
        let reg = registry();
        reg.subscribe("c1", ConnectionId(1));
        reg.subscribe("c1", ConnectionId(2));
        reg.unsubscribe("c1", ConnectionId(1));

        let subs = reg.subscribers("c1");
        assert_eq!(subs.len(), 1);
        assert!(subs.contains(&ConnectionId(2)));
    }

    #[test]
    fn unsubscribe_nonexistent_is_no_op() {
        let reg = registry();
        // Should not panic.
        reg.unsubscribe("c1", ConnectionId(99));
        assert!(reg.subscribers("c1").is_empty());
    }

    #[test]
    fn unsubscribe_all_removes_from_all_counters() {
        let reg = registry();
        reg.subscribe("c1", ConnectionId(1));
        reg.subscribe("c2", ConnectionId(1));
        reg.subscribe("c1", ConnectionId(2));

        reg.unsubscribe_all(ConnectionId(1));

        assert!(reg.subscribers("c1") == vec![ConnectionId(2)]);
        assert!(reg.subscribers("c2").is_empty());
    }

    #[test]
    fn subscribers_returns_empty_for_unknown_counter() {
        let reg = registry();
        assert!(reg.subscribers("unknown").is_empty());
    }

    #[test]
    fn duplicate_subscribe_is_idempotent() {
        let reg = registry();
        reg.subscribe("c1", ConnectionId(1));
        reg.subscribe("c1", ConnectionId(1));

        // DashSet deduplicates.
        assert_eq!(reg.subscribers("c1").len(), 1);
    }
}
