//! Journal store for mutation event audit/CDC streams.
//!
//! Maintains an in-memory ring buffer of `JournalEventData` entries with
//! subscription management. Events are appended with monotonically increasing
//! sequence numbers. Subscriptions track connection IDs and optional filters
//! (map name, event types).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use dashmap::DashMap;

use topgun_core::messages::{JournalEventData, JournalEventType};

use crate::network::connection::ConnectionId;

// ---------------------------------------------------------------------------
// JournalSubscription
// ---------------------------------------------------------------------------

/// A subscription to journal events with optional filters.
pub struct JournalSubscription {
    /// Connection that owns this subscription.
    pub connection_id: ConnectionId,
    /// Optional map name filter (only events for this map).
    pub map_name: Option<String>,
    /// Optional event type filter (only these event types).
    pub types: Option<Vec<JournalEventType>>,
}

// ---------------------------------------------------------------------------
// JournalStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory ring buffer for mutation events with subscription
/// management.
///
/// Events are stored in a `VecDeque` behind a `RwLock`, with an `AtomicU64`
/// for the monotonic sequence counter. The ring buffer has a fixed capacity;
/// when full, the oldest events are evicted.
pub struct JournalStore {
    /// Ring buffer of journal events.
    events: RwLock<VecDeque<JournalEventData>>,
    /// Monotonic sequence counter (starts at 1).
    next_sequence: AtomicU64,
    /// Maximum number of events to retain.
    capacity: usize,
    /// Active subscriptions keyed by subscription ID.
    subscriptions: DashMap<String, JournalSubscription>,
}

impl JournalStore {
    /// Creates a new journal store with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            events: RwLock::new(VecDeque::with_capacity(capacity)),
            next_sequence: AtomicU64::new(1),
            capacity,
            subscriptions: DashMap::new(),
        }
    }

    /// Appends an event to the ring buffer, assigning it a monotonic sequence
    /// number.
    ///
    /// Sets `event.sequence` to the string representation of the internal
    /// `u64` counter before storing (since `JournalEventData.sequence` is
    /// `String` on the wire but the store tracks sequences as `u64` internally).
    ///
    /// If the buffer is at capacity, the oldest event is evicted.
    ///
    /// Returns the assigned sequence number.
    pub fn append(&self, mut event: JournalEventData) -> u64 {
        let seq = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        event.sequence = seq.to_string();

        let mut events = self.events.write().expect("journal lock poisoned");
        if events.len() >= self.capacity {
            events.pop_front();
        }
        events.push_back(event);
        seq
    }

    /// Registers a subscription with the given ID.
    pub fn subscribe(&self, subscription_id: String, sub: JournalSubscription) {
        self.subscriptions.insert(subscription_id, sub);
    }

    /// Removes a subscription by ID.
    pub fn unsubscribe(&self, subscription_id: &str) {
        self.subscriptions.remove(subscription_id);
    }

    /// Removes all subscriptions for the given connection.
    pub fn unsubscribe_by_connection(&self, conn_id: ConnectionId) {
        self.subscriptions
            .retain(|_, sub| sub.connection_id != conn_id);
    }

    /// Reads events from the ring buffer starting at `from_sequence` with
    /// optional map name filter.
    ///
    /// Returns a tuple of `(events, has_more)` where `has_more` indicates
    /// whether additional events exist beyond the requested page.
    #[must_use]
    pub fn read(
        &self,
        from_sequence: u64,
        limit: u32,
        map_name: Option<&str>,
    ) -> (Vec<JournalEventData>, bool) {
        let events = self.events.read().expect("journal lock poisoned");

        let filtered: Vec<JournalEventData> = events
            .iter()
            .filter(|e| {
                // Parse stored sequence string back to u64 for comparison
                let seq: u64 = e.sequence.parse().unwrap_or(0);
                if seq < from_sequence {
                    return false;
                }
                // Apply optional map_name filter
                if let Some(name) = map_name {
                    if e.map_name != name {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        let total = filtered.len();
        let limit_usize = limit as usize;
        let has_more = total > limit_usize;
        let result = filtered.into_iter().take(limit_usize).collect();

        (result, has_more)
    }
}
