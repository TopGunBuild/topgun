//! Generic subscription registry for standing search subscriptions.
//!
//! Provides a single `SubscriptionRegistry<S>` that replaces both the former
//! `SearchRegistry` and `HybridSearchRegistry`, giving both subscription types
//! the full API surface (including `unregister_by_connection`,
//! `has_subscriptions_for_map`, and `has_any_subscriptions`).

use std::sync::Arc;

use dashmap::DashMap;

use crate::network::connection::ConnectionId;

// ---------------------------------------------------------------------------
// RegistryEntry trait
// ---------------------------------------------------------------------------

/// Trait bound for items stored in a [`SubscriptionRegistry`].
///
/// Implementors are the concrete subscription types (`SearchSubscription`,
/// `HybridSearchSubscription`). All methods return borrowed data from the
/// subscription struct — no heap allocation at the trait boundary.
pub trait RegistryEntry: Send + Sync + 'static {
    /// Unique identifier for this subscription.
    fn subscription_id(&self) -> &str;
    /// Connection that owns this subscription.
    fn connection_id(&self) -> ConnectionId;
    /// Name of the map this subscription targets.
    fn map_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// SubscriptionRegistry<S>
// ---------------------------------------------------------------------------

/// Generic concurrent subscription registry backed by [`DashMap`].
///
/// Keyed by the owning connection and the client-chosen `subscription_id`:
/// two connections may pick the same id, and neither may replace or remove the
/// other's subscription. Per-map iteration is O(n) where n is the total number
/// of subscriptions across all maps, acceptable for the expected workload (few
/// hundred concurrent subscriptions per server).
pub struct SubscriptionRegistry<S: RegistryEntry> {
    pub(crate) subscriptions: DashMap<(ConnectionId, String), Arc<S>>,
}

impl<S: RegistryEntry> SubscriptionRegistry<S> {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
        }
    }

    /// Registers a standing subscription, wrapping it in `Arc`.
    ///
    /// If the same connection already holds a subscription with the same ID
    /// it is replaced.
    pub fn register(&self, sub: S) {
        let key = (sub.connection_id(), sub.subscription_id().to_owned());
        self.subscriptions.insert(key, Arc::new(sub));
    }

    /// Removes `connection_id`'s subscription with the given ID.
    ///
    /// Returns the removed subscription, or `None` if that connection holds
    /// none with this ID.
    #[must_use]
    pub fn unregister(&self, connection_id: ConnectionId, subscription_id: &str) -> Option<Arc<S>> {
        self.subscriptions
            .remove(&(connection_id, subscription_id.to_owned()))
            .map(|(_, sub)| sub)
    }

    /// Removes all subscriptions for a given connection ID.
    ///
    /// Returns the IDs of all removed subscriptions. Called when a WebSocket
    /// connection closes so stale subscriptions are not left in the registry.
    #[must_use]
    pub fn unregister_by_connection(&self, connection_id: ConnectionId) -> Vec<String> {
        let mut removed = Vec::new();
        self.subscriptions.retain(|(owner, id), _| {
            if *owner == connection_id {
                removed.push(id.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// Returns all subscriptions targeting the given map.
    #[must_use]
    pub fn get_subscriptions_for_map(&self, map_name: &str) -> Vec<Arc<S>> {
        self.subscriptions
            .iter()
            .filter(|entry| entry.value().map_name() == map_name)
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    /// Returns true if any subscription targets the given map.
    ///
    /// O(n) scan returning early on first match. Negligible compared to a
    /// tantivy commit, which is the next operation in the hot path.
    #[must_use]
    pub fn has_subscriptions_for_map(&self, map_name: &str) -> bool {
        self.subscriptions
            .iter()
            .any(|entry| entry.value().map_name() == map_name)
    }

    /// Returns true if any subscriptions exist at all.
    #[must_use]
    pub fn has_any_subscriptions(&self) -> bool {
        !self.subscriptions.is_empty()
    }
}

impl<S: RegistryEntry> Default for SubscriptionRegistry<S> {
    fn default() -> Self {
        Self::new()
    }
}
