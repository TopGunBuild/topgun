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
                self.subscribers
                    .remove_if(name, |_, v| v.is_empty());
            }
        }
    }

    /// Removes a connection from all counter subscriber sets.
    ///
    /// Called on connection disconnect to clean up subscriptions.
    pub fn unsubscribe_all(&self, conn_id: ConnectionId) {
        let mut empty_keys = Vec::new();
        for entry in self.subscribers.iter() {
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

    /// Returns the current computed value of the named counter.
    ///
    /// Value = sum(p.values()) - sum(n.values()).
    /// Returns 0.0 if the counter does not exist.
    #[must_use]
    pub fn counter_value(&self, name: &str) -> f64 {
        self.counters
            .get(name)
            .map(|state| {
                let pos: f64 = state.p.values().sum();
                let neg: f64 = state.n.values().sum();
                pos - neg
            })
            .unwrap_or(0.0)
    }
}
