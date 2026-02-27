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
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned.
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
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::Timestamp;

    use super::*;

    fn make_event(map_name: &str, key: &str, event_type: JournalEventType) -> JournalEventData {
        JournalEventData {
            sequence: String::new(), // will be set by append()
            event_type,
            map_name: map_name.to_string(),
            key: key.to_string(),
            value: None,
            previous_value: None,
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 0,
                node_id: "test".to_string(),
            },
            node_id: "test".to_string(),
            metadata: None,
        }
    }

    // --- append ---

    #[test]
    fn append_assigns_monotonic_sequences() {
        let store = JournalStore::new(100);
        let s1 = store.append(make_event("m1", "k1", JournalEventType::PUT));
        let s2 = store.append(make_event("m1", "k2", JournalEventType::UPDATE));
        let s3 = store.append(make_event("m2", "k1", JournalEventType::DELETE));

        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[test]
    fn append_sets_sequence_string_on_event() {
        let store = JournalStore::new(100);
        store.append(make_event("m1", "k1", JournalEventType::PUT));

        let (events, _) = store.read(1, 100, None);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, "1");
    }

    #[test]
    fn append_evicts_oldest_when_at_capacity() {
        let store = JournalStore::new(3);
        store.append(make_event("m1", "k1", JournalEventType::PUT));
        store.append(make_event("m1", "k2", JournalEventType::PUT));
        store.append(make_event("m1", "k3", JournalEventType::PUT));
        store.append(make_event("m1", "k4", JournalEventType::PUT)); // evicts seq 1

        let (events, _) = store.read(1, 100, None);
        // Only seq 2, 3, 4 remain; seq 1 evicted.
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, "2");
        assert_eq!(events[2].sequence, "4");
    }

    // --- read ---

    #[test]
    fn read_from_sequence_filters_earlier_events() {
        let store = JournalStore::new(100);
        store.append(make_event("m1", "k1", JournalEventType::PUT));
        store.append(make_event("m1", "k2", JournalEventType::PUT));
        store.append(make_event("m1", "k3", JournalEventType::PUT));

        let (events, has_more) = store.read(2, 100, None);
        assert_eq!(events.len(), 2); // seq 2 and 3
        assert!(!has_more);
        assert_eq!(events[0].sequence, "2");
    }

    #[test]
    fn read_with_limit_returns_has_more() {
        let store = JournalStore::new(100);
        store.append(make_event("m1", "k1", JournalEventType::PUT));
        store.append(make_event("m1", "k2", JournalEventType::PUT));
        store.append(make_event("m1", "k3", JournalEventType::PUT));

        let (events, has_more) = store.read(1, 2, None);
        assert_eq!(events.len(), 2);
        assert!(has_more);
    }

    #[test]
    fn read_with_map_name_filter() {
        let store = JournalStore::new(100);
        store.append(make_event("users", "k1", JournalEventType::PUT));
        store.append(make_event("orders", "k1", JournalEventType::PUT));
        store.append(make_event("users", "k2", JournalEventType::UPDATE));

        let (events, _) = store.read(1, 100, Some("users"));
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.map_name == "users"));
    }

    #[test]
    fn read_empty_store_returns_empty() {
        let store = JournalStore::new(100);
        let (events, has_more) = store.read(1, 100, None);
        assert!(events.is_empty());
        assert!(!has_more);
    }

    #[test]
    fn read_from_beyond_last_sequence_returns_empty() {
        let store = JournalStore::new(100);
        store.append(make_event("m1", "k1", JournalEventType::PUT));

        let (events, has_more) = store.read(999, 100, None);
        assert!(events.is_empty());
        assert!(!has_more);
    }

    // --- subscribe / unsubscribe ---

    #[test]
    fn subscribe_and_unsubscribe() {
        let store = JournalStore::new(100);
        let sub = JournalSubscription {
            connection_id: ConnectionId(1),
            map_name: Some("users".to_string()),
            types: None,
        };
        store.subscribe("sub-1".to_string(), sub);

        // Verify subscription exists.
        assert!(store.subscriptions.contains_key("sub-1"));

        store.unsubscribe("sub-1");
        assert!(!store.subscriptions.contains_key("sub-1"));
    }

    #[test]
    fn unsubscribe_nonexistent_is_no_op() {
        let store = JournalStore::new(100);
        // Should not panic.
        store.unsubscribe("nonexistent");
    }

    #[test]
    fn unsubscribe_by_connection_removes_all_for_that_connection() {
        let store = JournalStore::new(100);
        store.subscribe(
            "sub-1".to_string(),
            JournalSubscription {
                connection_id: ConnectionId(1),
                map_name: None,
                types: None,
            },
        );
        store.subscribe(
            "sub-2".to_string(),
            JournalSubscription {
                connection_id: ConnectionId(1),
                map_name: None,
                types: None,
            },
        );
        store.subscribe(
            "sub-3".to_string(),
            JournalSubscription {
                connection_id: ConnectionId(2),
                map_name: None,
                types: None,
            },
        );

        store.unsubscribe_by_connection(ConnectionId(1));

        assert!(!store.subscriptions.contains_key("sub-1"));
        assert!(!store.subscriptions.contains_key("sub-2"));
        assert!(store.subscriptions.contains_key("sub-3"));
    }

    #[test]
    fn subscribe_with_filters() {
        let store = JournalStore::new(100);
        let sub = JournalSubscription {
            connection_id: ConnectionId(1),
            map_name: Some("users".to_string()),
            types: Some(vec![JournalEventType::PUT, JournalEventType::DELETE]),
        };
        store.subscribe("filtered-sub".to_string(), sub);

        let entry = store.subscriptions.get("filtered-sub").unwrap();
        assert_eq!(entry.map_name, Some("users".to_string()));
        assert_eq!(
            entry.types,
            Some(vec![JournalEventType::PUT, JournalEventType::DELETE])
        );
    }
}
