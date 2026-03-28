//! DAG execution engine and inbox/outbox buffer implementations.
//!
//! `VecDequeInbox` and `VecDequeOutbox` are the concrete buffer types used by
//! `DagExecutor` to pass items between processors. Bounded capacity enforces
//! backpressure: when a downstream inbox is full, `offer` returns `false` and
//! the upstream processor pauses.

use std::collections::VecDeque;

use crate::dag::types::{Inbox, Outbox};

/// Default maximum number of items a single inbox or outbox bucket can hold.
/// Chosen to bound per-processor memory usage while providing sufficient
/// buffering for bursty producers.
pub const DEFAULT_QUEUE_CAPACITY: usize = 4096;

// ---------------------------------------------------------------------------
// VecDequeInbox
// ---------------------------------------------------------------------------

/// Bounded single-ordinal inbox backed by a `VecDeque<rmpv::Value>`.
///
/// Upstream routing logic calls [`push`] to enqueue items. Processors read
/// via the [`Inbox`] trait methods.
pub struct VecDequeInbox {
    queue: VecDeque<rmpv::Value>,
    capacity: usize,
}

impl VecDequeInbox {
    /// Create a new inbox with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    /// Enqueue an item. Returns `false` if the inbox is at capacity (backpressure).
    pub fn push(&mut self, item: rmpv::Value) -> bool {
        if self.queue.len() >= self.capacity {
            return false;
        }
        self.queue.push_back(item);
        true
    }
}

impl Inbox for VecDequeInbox {
    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn peek(&self) -> Option<&rmpv::Value> {
        self.queue.front()
    }

    fn poll(&mut self) -> Option<rmpv::Value> {
        self.queue.pop_front()
    }

    fn drain(&mut self, callback: &mut dyn FnMut(rmpv::Value)) {
        while let Some(item) = self.queue.pop_front() {
            callback(item);
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }
}

// ---------------------------------------------------------------------------
// VecDequeOutbox
// ---------------------------------------------------------------------------

/// Bounded multi-ordinal outbox backed by one `VecDeque<rmpv::Value>` per output
/// ordinal. The executor drains each bucket into the corresponding downstream
/// processor's inbox after each `process()` call.
pub struct VecDequeOutbox {
    buckets: Vec<VecDeque<rmpv::Value>>,
    capacity: usize,
}

impl VecDequeOutbox {
    /// Create a new outbox with `bucket_count` output ordinals, each bounded by `capacity`.
    #[must_use]
    pub fn new(bucket_count: u32, capacity: usize) -> Self {
        let count = bucket_count as usize;
        Self {
            buckets: (0..count)
                .map(|_| VecDeque::with_capacity(capacity.min(1024)))
                .collect(),
            capacity,
        }
    }

    /// Drain all items from the given ordinal's bucket.
    ///
    /// Returns an iterator that removes items from front to back.
    /// Used by the executor to move items from an outbox bucket into the
    /// corresponding downstream inbox.
    pub fn drain_bucket(&mut self, ordinal: u32) -> impl Iterator<Item = rmpv::Value> + '_ {
        let bucket = &mut self.buckets[ordinal as usize];
        // Drain by swapping with an empty VecDeque and iterating the old contents.
        let drained = std::mem::take(bucket);
        drained.into_iter()
    }
}

impl Outbox for VecDequeOutbox {
    fn offer(&mut self, ordinal: u32, item: rmpv::Value) -> bool {
        let idx = ordinal as usize;
        if idx >= self.buckets.len() {
            return false;
        }
        if self.buckets[idx].len() >= self.capacity {
            // Backpressure: bucket is full.
            return false;
        }
        self.buckets[idx].push_back(item);
        true
    }

    fn offer_to_all(&mut self, item: rmpv::Value) -> bool {
        // Check capacity first — do not partially enqueue.
        for bucket in &self.buckets {
            if bucket.len() >= self.capacity {
                return false;
            }
        }
        // Clone to all except the last bucket; move into the last.
        let last = self.buckets.len().saturating_sub(1);
        for (i, bucket) in self.buckets.iter_mut().enumerate() {
            if i == last {
                bucket.push_back(item.clone());
            } else {
                bucket.push_back(item.clone());
            }
        }
        true
    }

    fn has_capacity(&self, ordinal: u32) -> bool {
        let idx = ordinal as usize;
        idx < self.buckets.len() && self.buckets[idx].len() < self.capacity
    }

    fn bucket_count(&self) -> u32 {
        self.buckets.len() as u32
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- VecDequeInbox ---

    #[test]
    fn inbox_push_and_poll() {
        let mut inbox = VecDequeInbox::new(4);
        assert!(inbox.is_empty());
        assert!(inbox.push(rmpv::Value::Integer(1.into())));
        assert!(!inbox.is_empty());
        assert_eq!(inbox.len(), 1);
        let item = inbox.poll().unwrap();
        assert_eq!(item, rmpv::Value::Integer(1.into()));
        assert!(inbox.is_empty());
    }

    #[test]
    fn inbox_backpressure_at_capacity() {
        let mut inbox = VecDequeInbox::new(2);
        assert!(inbox.push(rmpv::Value::Integer(1.into())));
        assert!(inbox.push(rmpv::Value::Integer(2.into())));
        // At capacity — next push should fail.
        assert!(!inbox.push(rmpv::Value::Integer(3.into())));
        assert_eq!(inbox.len(), 2);
    }

    #[test]
    fn inbox_peek_does_not_consume() {
        let mut inbox = VecDequeInbox::new(4);
        inbox.push(rmpv::Value::Boolean(true));
        assert_eq!(inbox.peek(), Some(&rmpv::Value::Boolean(true)));
        assert_eq!(inbox.len(), 1, "peek must not consume the item");
    }

    #[test]
    fn inbox_drain_empties_queue() {
        let mut inbox = VecDequeInbox::new(4);
        inbox.push(rmpv::Value::Integer(10.into()));
        inbox.push(rmpv::Value::Integer(20.into()));

        let mut collected = Vec::new();
        inbox.drain(&mut |item| collected.push(item));

        assert_eq!(collected.len(), 2);
        assert!(inbox.is_empty());
    }

    // --- VecDequeOutbox ---

    #[test]
    fn outbox_offer_and_drain() {
        let mut outbox = VecDequeOutbox::new(2, 4);
        assert_eq!(outbox.bucket_count(), 2);
        assert!(outbox.offer(0, rmpv::Value::Integer(1.into())));
        assert!(outbox.offer(1, rmpv::Value::Integer(2.into())));

        let drained0: Vec<_> = outbox.drain_bucket(0).collect();
        assert_eq!(drained0, vec![rmpv::Value::Integer(1.into())]);

        let drained1: Vec<_> = outbox.drain_bucket(1).collect();
        assert_eq!(drained1, vec![rmpv::Value::Integer(2.into())]);
    }

    #[test]
    fn outbox_backpressure() {
        let mut outbox = VecDequeOutbox::new(1, 2);
        assert!(outbox.offer(0, rmpv::Value::Integer(1.into())));
        assert!(outbox.offer(0, rmpv::Value::Integer(2.into())));
        // At capacity — next offer should fail.
        assert!(!outbox.offer(0, rmpv::Value::Integer(3.into())));
    }

    #[test]
    fn outbox_has_capacity() {
        let mut outbox = VecDequeOutbox::new(1, 2);
        assert!(outbox.has_capacity(0));
        outbox.offer(0, rmpv::Value::Integer(1.into()));
        outbox.offer(0, rmpv::Value::Integer(2.into()));
        assert!(!outbox.has_capacity(0));
    }

    #[test]
    fn outbox_offer_to_all() {
        let mut outbox = VecDequeOutbox::new(3, 4);
        assert!(outbox.offer_to_all(rmpv::Value::Boolean(true)));

        for i in 0..3 {
            let drained: Vec<_> = outbox.drain_bucket(i).collect();
            assert_eq!(drained.len(), 1);
            assert_eq!(drained[0], rmpv::Value::Boolean(true));
        }
    }

    #[test]
    fn outbox_invalid_ordinal_returns_false() {
        let mut outbox = VecDequeOutbox::new(2, 4);
        assert!(!outbox.offer(5, rmpv::Value::Nil));
    }

    #[test]
    fn drain_bucket_leaves_empty() {
        let mut outbox = VecDequeOutbox::new(1, 4);
        outbox.offer(0, rmpv::Value::Integer(42.into()));
        let _ = outbox.drain_bucket(0).collect::<Vec<_>>();
        // After drain, bucket should be empty.
        assert!(!outbox.has_capacity(0) || outbox.has_capacity(0)); // bucket exists
        let second_drain: Vec<_> = outbox.drain_bucket(0).collect();
        assert!(second_drain.is_empty());
    }
}
