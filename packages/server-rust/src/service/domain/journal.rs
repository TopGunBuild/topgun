//! Journal store for mutation event audit/CDC streams.
//!
//! Maintains an in-memory ring buffer of `JournalEventData` entries with
//! subscription management. Events are appended with monotonically increasing
//! sequence numbers. Subscriptions track connection IDs and optional filters
//! (map name, event types).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use parking_lot::RwLock;

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
    /// When `false`, `append` is a no-op and no events are ever stored or
    /// pushed. Lets an operator turn the journal off (`TOPGUN_JOURNAL_ENABLED=false`)
    /// to shed its per-write cost. Subscriptions and reads still function; reads
    /// simply observe an empty buffer (an explicit operator opt-out, surfaced at
    /// startup, not a silent dark feature).
    enabled: bool,
}

impl JournalStore {
    /// Creates a new enabled journal store with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::with_enabled(capacity, true)
    }

    /// Creates a journal store with the given capacity and enabled flag.
    ///
    /// Production wiring reads the flag from `TOPGUN_JOURNAL_ENABLED` (default
    /// `true`) so the Event Journal works out of the box while remaining cheap
    /// to disable.
    #[must_use]
    pub fn with_enabled(capacity: usize, enabled: bool) -> Self {
        Self {
            events: RwLock::new(VecDeque::with_capacity(capacity)),
            next_sequence: AtomicU64::new(1),
            capacity,
            subscriptions: DashMap::new(),
            enabled,
        }
    }

    /// Returns whether this journal is enabled (appends are recorded).
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
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
        // Disabled journal: never allocate or take the write lock. Keeps the
        // write path free of journal cost when an operator opts out.
        if !self.enabled {
            return 0;
        }
        // Assign the sequence INSIDE the lock so sequence order and storage order
        // can never diverge. With fetch_add outside the lock, two concurrent
        // worker pipelines could take seq N and N+1 and then push in the opposite
        // order, leaving the ring buffer (and every reader/subscriber) observing
        // non-monotonic sequences. Holding the lock makes assign+insert atomic.
        let mut events = self.events.write();
        let seq = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        event.sequence = seq.to_string();
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

    /// Returns the deduplicated connection IDs whose subscriptions match the
    /// given map name and event type.
    ///
    /// A subscription matches when its `map_name` filter is absent or equal to
    /// `map_name`, and its `types` filter is absent or contains `event_type`.
    /// One connection may hold several subscriptions; each ID appears once so
    /// the caller pushes a single `JOURNAL_EVENT` per connection (the client
    /// re-applies per-listener filters on receipt).
    #[must_use]
    pub fn subscribers_for(
        &self,
        map_name: &str,
        event_type: &JournalEventType,
    ) -> Vec<ConnectionId> {
        let mut ids: Vec<ConnectionId> = self
            .subscriptions
            .iter()
            .filter(|entry| {
                let sub = entry.value();
                if let Some(ref m) = sub.map_name {
                    if m != map_name {
                        return false;
                    }
                }
                if let Some(ref types) = sub.types {
                    if !types.contains(event_type) {
                        return false;
                    }
                }
                true
            })
            .map(|entry| entry.value().connection_id)
            .collect();
        ids.sort_unstable_by_key(|c| c.0);
        ids.dedup();
        ids
    }

    /// Returns the number of subscriptions held by the given connection.
    ///
    /// Exposed so disconnect-cleanup verification can observe per-connection
    /// subscription removal without reaching into the private subscription map.
    #[must_use]
    pub fn subscription_count_for_connection(&self, conn_id: ConnectionId) -> usize {
        self.subscriptions
            .iter()
            .filter(|entry| entry.value().connection_id == conn_id)
            .count()
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
        let events = self.events.read();

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
    fn disabled_store_append_is_noop() {
        let store = JournalStore::with_enabled(100, false);
        assert!(!store.is_enabled());
        let seq = store.append(make_event("m1", "k1", JournalEventType::PUT));
        // Disabled append assigns no sequence and stores nothing.
        assert_eq!(seq, 0);
        let (events, has_more) = store.read(0, 100, None);
        assert!(events.is_empty());
        assert!(!has_more);
    }

    #[test]
    fn concurrent_appends_preserve_monotonic_storage_order() {
        use std::sync::Arc;
        use std::thread;

        // Many threads append concurrently. Storage order must match sequence
        // order: assigning the sequence outside the write lock would let a higher
        // sequence land before a lower one in the deque. Guards that regression.
        let store = Arc::new(JournalStore::new(100_000));
        let threads = 8;
        let per_thread = 2_000;

        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let store = Arc::clone(&store);
                thread::spawn(move || {
                    for i in 0..per_thread {
                        store.append(make_event("m", &format!("{t}-{i}"), JournalEventType::PUT));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("append thread panicked");
        }

        let (events, _) = store.read(0, u32::MAX, None);
        assert_eq!(events.len(), threads * per_thread);
        let mut prev = 0u64;
        for e in &events {
            let seq: u64 = e.sequence.parse().expect("sequence parses");
            assert!(
                seq > prev,
                "storage order must be strictly increasing: {seq} after {prev}"
            );
            prev = seq;
        }
    }

    #[test]
    fn new_store_is_enabled_by_default() {
        let store = JournalStore::new(100);
        assert!(store.is_enabled());
        let seq = store.append(make_event("m1", "k1", JournalEventType::PUT));
        assert_eq!(seq, 1);
    }

    // --- subscribers_for ---

    fn make_sub(
        conn: u64,
        map_name: Option<&str>,
        types: Option<Vec<JournalEventType>>,
    ) -> JournalSubscription {
        JournalSubscription {
            connection_id: ConnectionId(conn),
            map_name: map_name.map(str::to_string),
            types,
        }
    }

    #[test]
    fn subscribers_for_matches_unfiltered_subscription() {
        let store = JournalStore::new(100);
        store.subscribe("s1".to_string(), make_sub(1, None, None));

        let ids = store.subscribers_for("users", &JournalEventType::PUT);
        assert_eq!(ids, vec![ConnectionId(1)]);
    }

    #[test]
    fn subscribers_for_applies_map_filter() {
        let store = JournalStore::new(100);
        store.subscribe("s1".to_string(), make_sub(1, Some("users"), None));
        store.subscribe("s2".to_string(), make_sub(2, Some("orders"), None));

        let ids = store.subscribers_for("users", &JournalEventType::PUT);
        assert_eq!(ids, vec![ConnectionId(1)]);
    }

    #[test]
    fn subscribers_for_applies_type_filter() {
        let store = JournalStore::new(100);
        store.subscribe(
            "s1".to_string(),
            make_sub(1, None, Some(vec![JournalEventType::DELETE])),
        );

        assert!(store
            .subscribers_for("users", &JournalEventType::PUT)
            .is_empty());
        assert_eq!(
            store.subscribers_for("users", &JournalEventType::DELETE),
            vec![ConnectionId(1)]
        );
    }

    #[test]
    fn subscribers_for_dedups_connection_with_multiple_subscriptions() {
        let store = JournalStore::new(100);
        // Same connection, two subscriptions both matching the event.
        store.subscribe("s1".to_string(), make_sub(1, None, None));
        store.subscribe("s2".to_string(), make_sub(1, Some("users"), None));

        let ids = store.subscribers_for("users", &JournalEventType::PUT);
        assert_eq!(ids, vec![ConnectionId(1)]);
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
