//! Generic subscription registry for standing search subscriptions.
//!
//! Provides a single `SubscriptionRegistry<S>` that replaces both the former
//! `SearchRegistry` and `HybridSearchRegistry`, giving both subscription types
//! the full API surface (including `unregister_by_connection`,
//! `has_subscriptions_for_map`, and `has_any_subscriptions`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;

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
// Per-map coverage
// ---------------------------------------------------------------------------

/// `covered_since` before the subscriber that ended the map's zero era has
/// read the write generation.
const NOT_READ: u64 = u64::MAX;

/// How one map is covered by the registry's subscriptions (`TG-LIVE-002`).
///
/// - `live`: the map's registered subscriptions, never less than them: it is
///   incremented before an insert and decremented after a removal.
/// - `covered_since`: the map's write generation read after the increment that
///   ended its last zero era ("coverage began"), or [`NOT_READ`] until a
///   subscriber has read it. Every write whose subscription check found
///   `live == 0` bumped the generation at or before that value.
///
/// The mutex is a leaf: nothing else is locked while it is held. The write path
/// never takes it.
#[derive(Default)]
pub(crate) struct MapCoverage {
    live: AtomicU64,
    covered_since: Mutex<u64>,
}

/// Held by every registry entry: dropping it decrements its map's `live`
/// count, so each removal path decrements exactly once.
pub(crate) struct CoverToken(Arc<MapCoverage>);

impl Drop for CoverToken {
    fn drop(&mut self) {
        let _since = self.0.covered_since.lock();
        // SeqCst: one half of the Dekker pair with the observer's generation
        // bump and count load. Downgrading it reopens the skipped-write race:
        // a write could read a stale non-zero count, be enqueued too late, or
        // be skipped without being counted by the next coverage.
        let previous = self.0.live.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous > 0, "coverage count underflow");
    }
}

/// A registered subscription and its share of its map's coverage count.
pub(crate) struct Registered<S> {
    pub(crate) sub: Arc<S>,
    _cover: CoverToken,
}

// ---------------------------------------------------------------------------
// SubscriptionRegistry<S>
// ---------------------------------------------------------------------------

/// Generic concurrent subscription registry backed by [`DashMap`].
///
/// Keyed by the owning connection and the client-chosen `subscription_id`:
/// two connections may pick the same id, and neither may replace or remove the
/// other's subscription. Listing a map's subscriptions is O(n) where n is the
/// total number of subscriptions across all maps; the per-write check
/// `has_subscriptions_for_map` is O(1), a per-map count kept by every insert and
/// removal.
pub struct SubscriptionRegistry<S: RegistryEntry> {
    pub(crate) subscriptions: DashMap<(ConnectionId, String), Registered<S>>,
    /// One entry per map ever registered, never removed (bounded by map names).
    coverage: DashMap<String, Arc<MapCoverage>>,
}

impl<S: RegistryEntry> SubscriptionRegistry<S> {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
            coverage: DashMap::new(),
        }
    }

    /// The coverage of `map_name`, created on first use. The shard guard is
    /// released before the caller locks the coverage mutex, which keeps that
    /// mutex a leaf.
    fn cover(&self, map_name: &str) -> Arc<MapCoverage> {
        if let Some(cover) = self.coverage.get(map_name) {
            return Arc::clone(cover.value());
        }
        Arc::clone(
            self.coverage
                .entry(map_name.to_owned())
                .or_default()
                .value(),
        )
    }

    /// Registers a standing subscription, wrapping it in `Arc`, and returns
    /// that `Arc`.
    ///
    /// If the same connection already holds a subscription with the same ID
    /// it is replaced. The map's coverage count is raised before the insert, so
    /// from here on a write to the map is enqueued for the index.
    // Callers that only register (tests) have no use for the `Arc`.
    #[allow(clippy::must_use_candidate)]
    pub fn register(&self, sub: S) -> Arc<S> {
        let cover = self.cover(sub.map_name());
        {
            let mut since = cover.covered_since.lock();
            // SeqCst: the subscriber's half of the Dekker pair with the
            // observer's generation bump and count load (`TG-LIVE-002`).
            // Downgrading it reopens the skipped-write race: a write could load
            // 0 after this increment and be skipped without being counted.
            if cover.live.fetch_add(1, Ordering::SeqCst) == 0 {
                // A new era: no reader may return the previous era's value.
                *since = NOT_READ;
            }
        }
        let sub = Arc::new(sub);
        let key = (sub.connection_id(), sub.subscription_id().to_owned());
        self.subscriptions.insert(
            key,
            Registered {
                sub: Arc::clone(&sub),
                _cover: CoverToken(cover),
            },
        );
        sub
    }

    /// The write generation from which `map_name` has been covered by this
    /// registry's subscriptions without a gap. The first call of an era reads
    /// `generation` (after the increment that began the era) and every later
    /// call of the era returns that value. Every write that was skipped for
    /// lack of a subscription bumped `generation` at or before it.
    pub(crate) fn covered_since(&self, map_name: &str, generation: &AtomicU64) -> u64 {
        let cover = self.cover(map_name);
        let mut since = cover.covered_since.lock();
        if *since == NOT_READ {
            // SeqCst: read after the era's `live` increment in the single
            // order of SeqCst operations. Downgrading it reopens the
            // skipped-write race: it could miss the bump of a write that
            // loaded `live == 0`.
            *since = generation.load(Ordering::SeqCst);
        }
        *since
    }

    /// Removes `connection_id`'s subscription with the given ID.
    ///
    /// Returns the removed subscription, or `None` if that connection holds
    /// none with this ID.
    #[must_use]
    pub fn unregister(&self, connection_id: ConnectionId, subscription_id: &str) -> Option<Arc<S>> {
        self.subscriptions
            .remove(&(connection_id, subscription_id.to_owned()))
            .map(|(_, entry)| Arc::clone(&entry.sub))
    }

    /// Removes `connection_id`'s subscription with the given ID only if it is
    /// `sub` itself, not a later registration of the same `(connection, id)`.
    /// Returns whether it removed it.
    pub fn unregister_if_same(
        &self,
        connection_id: ConnectionId,
        subscription_id: &str,
        sub: &Arc<S>,
    ) -> bool {
        self.subscriptions
            .remove_if(&(connection_id, subscription_id.to_owned()), |_, entry| {
                Arc::ptr_eq(&entry.sub, sub)
            })
            .is_some()
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
            .filter(|entry| entry.value().sub.map_name() == map_name)
            .map(|entry| Arc::clone(&entry.value().sub))
            .collect()
    }

    /// Returns true if any subscription targets the given map.
    ///
    /// O(1): one shard read lock of the coverage map and one load of the map's
    /// count, which is never below its registered subscriptions.
    #[must_use]
    pub fn has_subscriptions_for_map(&self, map_name: &str) -> bool {
        self.coverage.get(map_name).is_some_and(|cover| {
            // SeqCst: the writer's half of the Dekker pair; the observer bumps
            // the generation (SeqCst) before this load. Downgrading it reopens
            // the skipped-write race of `TG-LIVE-002`.
            cover.live.load(Ordering::SeqCst) > 0
        })
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
