//! `BinaryHeap`-backed sorted page collector for full-scan queries.
//!
//! [`FullScanPager`] materialises the top-`limit` rows from an unbounded scan
//! without loading the whole dataset into memory. It maintains a bounded
//! [`BinaryHeap`] of capacity `limit + 1`:
//!
//! - The extra `+1` slot is the pagination sentinel: if the heap ever holds
//!   more than `limit + 1` rows, the worst entry is evicted immediately (O(log n)
//!   bounded to O(log(limit+1))). After the scan the sentinel position tells the
//!   caller whether a next page exists.
//! - Entries that compare equal on every user-supplied sort field are broken by
//!   the record's primary key in ascending order (via [`SortKey::_key`]) so the
//!   result is deterministic regardless of insertion order, eviction cycles, or
//!   batch boundaries.
//!
//! ### Cap = limit + 1, not limit
//!
//! The (limit+1)-th entry is kept alive during the scan specifically to detect
//! overflow: if the heap retains `limit + 1` entries after scanning all available
//! rows, at least one row exists beyond the current page, so `has_more = true`.
//! That sentinel entry is then discarded before the result is returned so the
//! caller always receives at most `limit` rows.
//!
//! ### Usage
//!
//! ```rust,ignore
//! let mut pager = FullScanPager::new(&sort_spec, limit);
//! for (key, value) in rows {
//!     pager.push(&key, &value);
//! }
//! let (page, has_more) = pager.finish();
//! // `page` contains at most `limit` rows in sort order; `has_more` is true when
//! // more rows exist beyond this page.
//! ```

use std::collections::BinaryHeap;

use topgun_core::messages::base::SortDirection;

use crate::query::sort_key::SortKey;

// ---------------------------------------------------------------------------
// HeapEntry
// ---------------------------------------------------------------------------

/// A heap entry pairing a `SortKey` with the record value it represents.
///
/// `BinaryHeap` requires `Ord` on its element type. `rmpv::Value` does not
/// implement `Ord`, so we wrap the pair in a newtype that delegates `Ord`
/// entirely to the `SortKey` and ignores the value for ordering purposes.
#[derive(Debug, Clone)]
struct HeapEntry {
    key: SortKey,
    value: rmpv::Value,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// FullScanPager
// ---------------------------------------------------------------------------

/// A bounded, sort-aware page collector for full-scan queries.
///
/// See the module documentation for the `cap=limit+1` semantics and tie-break rules.
pub struct FullScanPager {
    /// Sort spec aligned to `Query.sort`.
    sort_spec: Vec<(String, SortDirection)>,
    /// Maximum number of rows to return.
    limit: usize,
    /// Internal `BinaryHeap`. Capacity is `limit + 1`; the `+1` slot is the
    /// pagination sentinel that is discarded by `finish()`.
    heap: BinaryHeap<HeapEntry>,
}

impl FullScanPager {
    /// Creates a new pager for a given sort spec and page limit.
    ///
    /// `sort_spec` is a list of `(field_name, direction)` pairs aligned to
    /// `Query.sort`. An empty list means key-only ordering.
    ///
    /// `limit` is the maximum number of rows the caller wants. The internal
    /// heap cap is `limit + 1` so a full heap proves there is a next page.
    #[must_use]
    pub fn new(sort_spec: Vec<(String, SortDirection)>, limit: usize) -> Self {
        Self {
            sort_spec,
            limit,
            heap: BinaryHeap::new(),
        }
    }

    /// Pushes one `(key, value)` row into the pager.
    ///
    /// If the heap already holds `limit + 1` rows (the sentinel capacity) and the
    /// new row sorts better than the current worst row, the worst row is evicted
    /// and the new row takes its place. Rows that would sort AFTER all current
    /// rows (i.e. worse than the worst in a full heap) are immediately discarded.
    ///
    /// This is O(log(limit)) per push, bounded entirely by `limit`, not by the
    /// total number of rows scanned.
    pub fn push(&mut self, key: &str, value: &rmpv::Value) {
        let cap = self.limit + 1;
        let sort_key = SortKey::new(&self.sort_spec, value, key);

        // When the heap is full and the new row sorts worse than the current
        // worst (heap root in the inverted max-heap), discard it immediately
        // rather than pushing and popping — avoids the redundant heap operation.
        if self.heap.len() >= cap {
            if let Some(root) = self.heap.peek() {
                // `SortKey::Ord` is inverted so the heap root has the WORST
                // natural order. If the new key is also worse-or-equal to the
                // root, skip it.
                if sort_key.natural_cmp(&root.key) != std::cmp::Ordering::Less {
                    return;
                }
            }
        }

        self.heap.push(HeapEntry {
            key: sort_key,
            value: value.clone(),
        });

        // Evict the worst row when cap is exceeded so the heap never grows beyond
        // limit + 1 entries (bounded memory, bounded log-factor per operation).
        if self.heap.len() > cap {
            self.heap.pop();
        }
    }

    /// Finalises the page and returns `(rows, has_more)`.
    ///
    /// - `rows` contains at most `limit` entries in ascending sort order (the
    ///   natural page order, best-first).
    /// - `has_more` is `true` when the heap held exactly `limit + 1` entries at
    ///   finalisation time, proving at least one row exists beyond the page.
    ///
    /// The sentinel entry is discarded before returning: if `has_more` is `true`,
    /// the last row popped from the heap (which would be the (limit+1)-th in page
    /// order) is NOT included in `rows`.
    ///
    /// Consumes `self`.
    #[must_use]
    pub fn finish(mut self) -> (Vec<(String, rmpv::Value)>, bool) {
        let has_more = self.heap.len() > self.limit;

        // Drain the heap; `BinaryHeap::pop()` gives worst-first (max-heap root is worst),
        // so collecting into a Vec gives worst-last. After reversing, the result
        // is in natural page order (best-first / ascending by sort fields).
        let mut items: Vec<(String, rmpv::Value)> = Vec::with_capacity(self.heap.len());
        while let Some(entry) = self.heap.pop() {
            #[allow(clippy::used_underscore_binding)]
            items.push((entry.key._key, entry.value));
        }
        items.reverse();

        // Discard the sentinel: if has_more, the last entry in `items` (the worst
        // row that proved overflow) must not be returned to the caller.
        if has_more {
            items.truncate(self.limit);
        }

        (items, has_more)
    }

    /// Drains the pager into ascending page order (best-first), KEEPING the
    /// `(limit + 1)`-th sentinel row when the heap is full.
    ///
    /// Unlike [`finish`](Self::finish), the sentinel is NOT discarded. The
    /// streaming `ScanProcessor` source forwards up to `limit + 1` rows so a
    /// downstream `LimitProcessor` (configured with `limit + 1`) and the query
    /// emission site detect `has_more` from the row count exactly as they do for
    /// the in-memory DAG path — the bounded source stays wire-compatible with the
    /// existing pagination contract instead of inventing a second has-more channel.
    #[must_use]
    pub fn into_sorted_rows(mut self) -> Vec<(String, rmpv::Value)> {
        let mut items: Vec<(String, rmpv::Value)> = Vec::with_capacity(self.heap.len());
        while let Some(entry) = self.heap.pop() {
            #[allow(clippy::used_underscore_binding)]
            items.push((entry.key._key, entry.value));
        }
        items.reverse();
        items
    }

    /// Returns the current number of entries in the heap (for testing and observability).
    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns `true` when the heap is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use topgun_core::messages::base::SortDirection;

    use super::*;

    fn int_rec(field: &str, n: i64) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(field.into()),
            rmpv::Value::Integer(n.into()),
        )])
    }

    fn spec_asc(field: &str) -> Vec<(String, SortDirection)> {
        vec![(field.to_string(), SortDirection::Asc)]
    }

    fn spec_desc(field: &str) -> Vec<(String, SortDirection)> {
        vec![(field.to_string(), SortDirection::Desc)]
    }

    fn extract_int(v: &rmpv::Value) -> i64 {
        if let rmpv::Value::Map(pairs) = v {
            if let rmpv::Value::Integer(n) = &pairs[0].1 {
                return n.as_i64().unwrap_or(0);
            }
        }
        0
    }

    /// cap=limit+1: pager with limit=3 and 4 insertions emits exactly 3 rows
    /// and `has_more=true`.
    #[test]
    fn cap_limit_plus_one_has_more_true_when_overflow() {
        let mut pager = FullScanPager::new(spec_asc("v"), 3);
        for (key, val) in &[("k1", 10), ("k2", 20), ("k3", 30), ("k4", 40)] {
            pager.push(key, &int_rec("v", *val));
        }
        let (rows, has_more) = pager.finish();
        assert!(has_more, "4 rows for limit=3 must signal has_more");
        assert_eq!(rows.len(), 3, "must emit exactly limit rows");
        // Page order: 10, 20, 30 (ascending).
        let vals: Vec<i64> = rows.iter().map(|(_, v)| extract_int(v)).collect();
        assert_eq!(vals, vec![10, 20, 30]);
    }

    /// cap=limit+1: pager with exactly limit insertions emits limit rows and
    /// `has_more=false` (no sentinel overflow).
    #[test]
    fn cap_limit_plus_one_has_more_false_when_exact_limit() {
        let mut pager = FullScanPager::new(spec_asc("v"), 3);
        for (key, val) in &[("k1", 10), ("k2", 20), ("k3", 30)] {
            pager.push(key, &int_rec("v", *val));
        }
        let (rows, has_more) = pager.finish();
        assert!(!has_more, "exactly limit rows must not signal has_more");
        assert_eq!(rows.len(), 3);
    }

    /// Pager with fewer rows than limit: all rows returned, `has_more=false`.
    #[test]
    fn fewer_rows_than_limit_returns_all() {
        let mut pager = FullScanPager::new(spec_asc("v"), 10);
        pager.push("k1", &int_rec("v", 5));
        pager.push("k2", &int_rec("v", 3));
        let (rows, has_more) = pager.finish();
        assert!(!has_more);
        assert_eq!(rows.len(), 2);
    }

    /// Rows are returned in ascending sort order (best-first).
    #[test]
    fn rows_in_ascending_order() {
        let mut pager = FullScanPager::new(spec_asc("v"), 5);
        // Insert in reverse order to verify the pager sorts.
        for (key, val) in &[("k5", 50), ("k3", 30), ("k1", 10), ("k4", 40), ("k2", 20)] {
            pager.push(key, &int_rec("v", *val));
        }
        let (rows, _) = pager.finish();
        let vals: Vec<i64> = rows.iter().map(|(_, v)| extract_int(v)).collect();
        assert_eq!(vals, vec![10, 20, 30, 40, 50]);
    }

    /// DESC sort: largest values first.
    #[test]
    fn desc_sort_returns_largest_first() {
        let mut pager = FullScanPager::new(spec_desc("v"), 3);
        for (key, val) in &[("k1", 10), ("k2", 50), ("k3", 30), ("k4", 20), ("k5", 40)] {
            pager.push(key, &int_rec("v", *val));
        }
        let (rows, has_more) = pager.finish();
        assert!(has_more);
        assert_eq!(rows.len(), 3);
        let vals: Vec<i64> = rows.iter().map(|(_, v)| extract_int(v)).collect();
        assert_eq!(vals, vec![50, 40, 30]);
    }

    /// `_key` tie-break: when sort values are equal, the row with the
    /// lexicographically smaller key appears first on the page.
    #[test]
    fn key_tiebreak_smaller_key_appears_first() {
        let mut pager = FullScanPager::new(spec_asc("v"), 3);
        // All have v=5; keys c, a, b should come out as a, b, c.
        pager.push("c", &int_rec("v", 5));
        pager.push("a", &int_rec("v", 5));
        pager.push("b", &int_rec("v", 5));
        let (rows, _) = pager.finish();
        assert_eq!(rows.len(), 3);
        let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    /// `_key` tie-break: with overflow, the row with the largest key (worst) is
    /// evicted, keeping the two with smallest keys.
    #[test]
    fn key_tiebreak_evicts_largest_key_on_overflow() {
        let mut pager = FullScanPager::new(spec_asc("v"), 2);
        // limit=2, cap=3. Insert a, b, c all with v=5.
        // Heap at cap 3: a, b, c. On 4th push it's 4 > cap → evict worst ("d"
        // since d > c > b > a).
        for key in &["c", "a", "b", "d"] {
            pager.push(key, &int_rec("v", 5));
        }
        let (rows, has_more) = pager.finish();
        // 4 items, cap=3, heap retains 3 before finish → `has_more=true`.
        assert!(has_more);
        // `finish()` discards the sentinel; only limit=2 rows returned.
        assert_eq!(rows.len(), 2);
        let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        // The two best (smallest keys): "a" and "b".
        assert_eq!(keys, vec!["a", "b"]);
    }

    /// Empty pager: `finish()` returns empty vec and `has_more=false`.
    #[test]
    fn empty_pager_finish_returns_empty() {
        let pager = FullScanPager::new(spec_asc("v"), 5);
        let (rows, has_more) = pager.finish();
        assert!(rows.is_empty());
        assert!(!has_more);
    }

    /// Key-only sort (empty spec): rows are returned in key order.
    #[test]
    fn key_only_sort_order() {
        let mut pager = FullScanPager::new(vec![], 5);
        for key in &["dog", "cat", "ant", "bee"] {
            pager.push(key, &rmpv::Value::Nil);
        }
        let (rows, _) = pager.finish();
        let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["ant", "bee", "cat", "dog"]);
    }
}
