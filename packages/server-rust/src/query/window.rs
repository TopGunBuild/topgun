//! Live windowed top-N maintenance for standing query subscriptions.
//!
//! A [`LiveWindow`] tracks the in-window membership of one subscription with a `limit`
//! (sorted top-N). When a mutation arrives it returns the COMPLETE set of
//! [`WindowDelta`]s the subscriber must receive — including displacement `LEAVE`s (an
//! `ENTER` pushing the (limit+1)-th row out) and promotion `ENTER`s (a freed slot pulling
//! a previously-displaced row back in).
//!
//! Ordering is delegated to the shared cursor comparator in [`crate::query::cursor`] so
//! live ordering and keyset-cursor ordering cannot diverge: a row is encoded as a
//! [`CursorData`] (its sort-field values plus its key as the tie-break) and rows are
//! compared via [`is_after_cursor`].

use std::collections::HashMap;

use parking_lot::Mutex;
use topgun_core::messages::base::{ChangeEventType, SortField};

use crate::query::cursor::{is_after_cursor, rmpv_to_json_value, CursorData, SortValue};

// ---------------------------------------------------------------------------
// WindowDelta
// ---------------------------------------------------------------------------

/// A single change the subscriber must be told about for one mutation.
///
/// Server-internal: it carries the resolved record value and the change classification.
/// `value` is [`rmpv::Value::Nil`] for a `LEAVE` (the subscriber only needs the key to
/// drop the row).
#[derive(Debug, Clone)]
pub struct WindowDelta {
    /// Record key the delta applies to.
    pub key: String,
    /// Resolved record value (Nil for a `LEAVE`).
    pub value: rmpv::Value,
    /// Whether the row entered, updated within, or left the window.
    pub event: ChangeEventType,
}

// ---------------------------------------------------------------------------
// Window row
// ---------------------------------------------------------------------------

/// A row the window has SEEN (currently in-window or retained-out-of-window).
///
/// Retaining out-of-window rows that were once displaced is what lets a freed slot
/// re-promote the correct previously-displaced row without re-querying the DAG (see the
/// promotion-data bound on [`LiveWindow::apply_mutation`]).
#[derive(Debug, Clone)]
struct WindowRow {
    key: String,
    value: rmpv::Value,
}

// ---------------------------------------------------------------------------
// LiveWindow
// ---------------------------------------------------------------------------

/// Interior-mutable sorted top-N window for one subscription.
///
/// `QuerySubscription` is `Arc`-stored with no `&mut` path, so all mutable state lives
/// behind a [`Mutex`] and [`apply_mutation`](Self::apply_mutation) takes `&self`.
pub struct LiveWindow {
    /// Sort spec aligned to `Query.sort` (empty = key-only ordering).
    sort: Vec<SortField>,
    /// Window size. `None` = unbounded: deltas pass through predicate-only with no
    /// displacement (behavior unchanged from a pre-windowed subscription).
    limit: Option<u32>,
    /// All rows the window has seen and still tracks, ordered by the keyset comparator.
    ///
    /// The first `limit` entries (by sort order) are the in-window rows; any trailing
    /// entries are retained out-of-window candidates available for promotion.
    state: Mutex<Vec<WindowRow>>,
    /// Per-key last-seen HLC: `(millis, counter)` from the most recent accepted write.
    ///
    /// Guards the full-scan merge path against stale datastore pages overwriting newer
    /// values that already arrived via live CRDT writes. A record whose HLC compares
    /// `<=` to the stored value is silently dropped; one with a strictly greater HLC
    /// is accepted and the stored value is updated. The live `apply_mutation` path
    /// (which does not carry HLC) always passes through without consulting this map —
    /// the guard is only active when an explicit HLC is supplied via
    /// `apply_mutation_with_hlc`.
    hlc_seen: Mutex<HashMap<String, (u64, u32)>>,
    /// The page cursor of a subscription that pages from one: the window tracks
    /// no row that sorts at or before it, so a row moved there leaves and a row
    /// written there takes no slot of the page.
    page_cursor: Option<CursorData>,
}

impl LiveWindow {
    /// Builds a window from a subscription's sort spec and limit.
    #[must_use]
    pub fn new(sort: Vec<SortField>, limit: Option<u32>) -> Self {
        Self {
            sort,
            limit,
            state: Mutex::new(Vec::new()),
            hlc_seen: Mutex::new(HashMap::new()),
            page_cursor: None,
        }
    }

    /// The same window for a page that follows `cursor`.
    #[must_use]
    pub fn after_cursor(mut self, cursor: CursorData) -> Self {
        self.page_cursor = Some(cursor);
        self
    }

    /// Encodes a row as a [`CursorData`] so [`is_after_cursor`] can order it against
    /// other rows using the exact per-field ASC/DESC + `last_key` tie-break semantics the
    /// keyset cursor uses.
    fn row_cursor(&self, key: &str, value: &rmpv::Value) -> CursorData {
        let sort_values: Vec<SortValue> = self
            .sort
            .iter()
            .map(|sf| {
                let field_val = match value {
                    rmpv::Value::Map(pairs) => pairs
                        .iter()
                        .find(|(k, _)| k.as_str() == Some(sf.field.as_str()))
                        .map_or(serde_json::Value::Null, |(_, v)| {
                            rmpv_to_json_value(v).unwrap_or(serde_json::Value::Null)
                        }),
                    _ => serde_json::Value::Null,
                };
                SortValue {
                    field: sf.field.clone(),
                    value: field_val,
                    direction: sf.direction.clone(),
                }
            })
            .collect();

        CursorData {
            sort_values,
            last_key: key.to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        }
    }

    /// Returns `true` when row `a` sorts strictly BEFORE row `b` under the window order.
    ///
    /// Implemented as "b comes after a" via the shared cursor comparator so a single
    /// ordering definition governs both pagination and live windowing.
    fn row_before(&self, a: (&str, &rmpv::Value), b: (&str, &rmpv::Value)) -> bool {
        let a_cursor = self.row_cursor(a.0, a.1);
        is_after_cursor(b.0, b.1, &a_cursor)
    }

    /// Applies several mutations at once and returns every delta the subscriber
    /// must receive for their net effect: the state they leave is the state the
    /// same mutations leave applied one by one, with the window sorted and
    /// diffed once instead of once per mutation. A live write is a batch of
    /// one ([`apply_mutation`](Self::apply_mutation)); a snapshot's initial
    /// load and its captured writes are one batch each. The last mutation of a key
    /// decides it. A key that leaves and comes back within the batch is one
    /// `UPDATE` (or nothing, outside the window), not a `LEAVE` and an `ENTER`.
    ///
    /// Out-of-window rows are trimmed once, at the end, so a batch can keep a
    /// retained row that the same mutations applied one by one would have
    /// trimmed on the way (see the promotion-data bound on
    /// [`apply_mutation`](Self::apply_mutation)).
    pub fn apply_batch<'a>(
        &self,
        mutations: impl IntoIterator<Item = (&'a str, Option<&'a rmpv::Value>, bool)>,
    ) -> Vec<WindowDelta> {
        // The value each mutated key ends with; `None` = no row (delete,
        // predicate-false, or a row at or before the page cursor).
        let mut last: HashMap<&str, Option<&rmpv::Value>> = HashMap::new();
        for (key, value, matches_predicate) in mutations {
            let after = |value: &&rmpv::Value| {
                self.page_cursor
                    .as_ref()
                    .is_none_or(|cursor| is_after_cursor(key, value, cursor))
            };
            last.insert(key, value.filter(|v| matches_predicate && after(v)));
        }
        let window = self.limit.map_or(usize::MAX, |limit| limit as usize);
        let mut state = self.state.lock();
        // Removals of keys the window does not track change nothing: the common
        // case of a write that does not match a live query.
        let adds = last.values().any(Option::is_some);
        let only = (last.len() == 1)
            .then(|| last.keys().next().copied())
            .flatten();
        let tracked = |row: &WindowRow| {
            only.map_or_else(|| last.contains_key(row.key.as_str()), |key| row.key == key)
        };
        if !adds && !state.iter().any(tracked) {
            return Vec::new();
        }
        let (mut leaves, mut enters, mut updates) = (Vec::new(), Vec::new(), Vec::new());
        // Each row with whether it was in the window and whether it was mutated.
        let mut rows: Vec<(WindowRow, bool, bool)> = Vec::with_capacity(state.len() + last.len());
        for (i, row) in state.drain(..).enumerate() {
            let was_in = i < window;
            match last.remove(row.key.as_str()) {
                None => rows.push((row, was_in, false)),
                Some(Some(value)) => rows.push((
                    WindowRow {
                        key: row.key,
                        value: value.clone(),
                    },
                    was_in,
                    true,
                )),
                Some(None) if was_in => leaves.push(WindowDelta {
                    key: row.key,
                    value: rmpv::Value::Nil,
                    event: ChangeEventType::LEAVE,
                }),
                Some(None) => {}
            }
        }
        for (key, value) in last {
            if let Some(value) = value {
                let row = WindowRow {
                    key: key.to_string(),
                    value: value.clone(),
                };
                rows.push((row, false, true));
            }
        }
        // Removals keep the rows in order; only a new or changed value moves one.
        if adds {
            rows.sort_by(|(a, ..), (b, ..)| self.compare((&a.key, &a.value), (&b.key, &b.value)));
        }

        for (i, (row, was_in, mutated)) in rows.iter().enumerate() {
            match (*was_in, i < window) {
                (true, false) => leaves.push(WindowDelta {
                    key: row.key.clone(),
                    value: rmpv::Value::Nil,
                    event: ChangeEventType::LEAVE,
                }),
                (false, true) => enters.push(WindowDelta {
                    key: row.key.clone(),
                    value: row.value.clone(),
                    event: ChangeEventType::ENTER,
                }),
                (true, true) if *mutated => updates.push(WindowDelta {
                    key: row.key.clone(),
                    value: row.value.clone(),
                    event: ChangeEventType::UPDATE,
                }),
                _ => {}
            }
        }

        // Retain only enough out-of-window candidates to refill freed slots after
        // later removals: `limit` beyond the window.
        if let Some(limit) = self.limit {
            rows.truncate((limit as usize).saturating_mul(2));
        }
        *state = rows.into_iter().map(|(row, ..)| row).collect();
        leaves.extend(enters);
        leaves.extend(updates);
        leaves
    }

    /// Whether the window tracks rows past its limit: rows a write pushed out of
    /// the window, which still match and follow its last row.
    pub fn holds_past_limit(&self) -> bool {
        self.limit
            .is_some_and(|limit| self.state.lock().len() > limit as usize)
    }

    /// Each tracked key's position in the window's order: the keyset
    /// comparator over the unprojected values, the key breaking ties.
    pub fn ranks(&self) -> HashMap<String, usize> {
        self.state
            .lock()
            .iter()
            .enumerate()
            .map(|(i, row)| (row.key.clone(), i))
            .collect()
    }

    /// The window's order of two `(key, value)` rows: the keyset comparator over
    /// its sort fields, the key breaking ties.
    fn compare(&self, a: (&str, &rmpv::Value), b: (&str, &rmpv::Value)) -> std::cmp::Ordering {
        if self.row_before(a, b) {
            std::cmp::Ordering::Less
        } else if self.row_before(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }

    /// Applies one mutation and returns every delta the subscriber must receive.
    ///
    /// `new_value = None` models a delete/tombstone. `matches_predicate` is the caller's
    /// already-computed predicate result for `new_value` (false for a delete).
    ///
    /// Semantics:
    /// - New matching row sorting inside top-N → `ENTER`; if the window was full, the row
    ///   now (limit+1)-th leaves via a displacement `LEAVE`.
    /// - Row leaves (delete / predicate-false / displaced) → `LEAVE`; a freed slot
    ///   re-promotes the best retained out-of-window row via `ENTER`.
    /// - Row updated still inside top-N → `UPDATE` (window re-sorted; reorder is conveyed
    ///   by `UPDATE`, there is no `MOVE` variant). An update that pushes the row out of
    ///   top-N is a displacement `LEAVE` + promotion `ENTER`.
    /// - Unbounded (`limit = None`) → predicate-only `ENTER`/`UPDATE`/`LEAVE`, never a
    ///   displacement.
    ///
    /// Promotion-data bound: the window can only re-promote rows it has SEEN enter/leave,
    /// because the DAG produced only the current page — out-of-window candidates are not
    /// all resident. When a slot frees and no retained candidate exists, no synthetic
    /// `ENTER` is emitted (the next matching write fills the slot). This is correct for the
    /// displacement case this routine targets (a 3rd row pushing a 2nd out, then the 3rd
    /// leaving re-promoting the 2nd); it is NOT a full re-query-on-delete.
    pub fn apply_mutation(
        &self,
        key: &str,
        new_value: Option<&rmpv::Value>,
        matches_predicate: bool,
    ) -> Vec<WindowDelta> {
        self.apply_batch(std::iter::once((key, new_value, matches_predicate)))
    }

    /// Applies one mutation with an explicit HLC monotonicity guard.
    ///
    /// This variant is used by the full-scan datastore path where records may arrive
    /// out of HLC order (batched pages from the durable store, interleaved with live
    /// CRDT writes). If the supplied `hlc_millis`/`hlc_counter` pair is less than or
    /// equal to the last-seen HLC for `key`, the record is silently dropped and an
    /// empty delta set is returned. A strictly greater HLC is accepted, the stored
    /// last-seen value is advanced, and the call falls through to the standard
    /// [`apply_mutation`](Self::apply_mutation) logic.
    ///
    /// Live CRDT writes that do not carry a datastore HLC should use the plain
    /// `apply_mutation` method, which bypasses the monotone guard entirely.
    pub fn apply_mutation_with_hlc(
        &self,
        key: &str,
        new_value: Option<&rmpv::Value>,
        matches_predicate: bool,
        hlc_millis: u64,
        hlc_counter: u32,
    ) -> Vec<WindowDelta> {
        let incoming = (hlc_millis, hlc_counter);

        // Guard: silently drop records whose HLC is not strictly greater than the
        // last-seen HLC for this key. This prevents stale datastore pages from
        // clobbering newer in-memory values that arrived via live CRDT writes.
        {
            let mut seen = self.hlc_seen.lock();
            match seen.get(key) {
                Some(&last) if incoming <= last => return Vec::new(),
                _ => {
                    seen.insert(key.to_string(), incoming);
                }
            }
        }

        self.apply_mutation(key, new_value, matches_predicate)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::base::SortDirection;

    fn sort_asc(field: &str) -> Vec<SortField> {
        vec![SortField {
            field: field.to_string(),
            direction: SortDirection::Asc,
        }]
    }

    /// Builds a record `{ field: int_value }`.
    fn rec(field: &str, v: i64) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(field.into()),
            rmpv::Value::Integer(v.into()),
        )])
    }

    fn events(deltas: &[WindowDelta]) -> Vec<(String, ChangeEventType)> {
        deltas
            .iter()
            .map(|d| (d.key.clone(), d.event.clone()))
            .collect()
    }

    fn has(deltas: &[WindowDelta], key: &str, event: &ChangeEventType) -> bool {
        deltas.iter().any(|d| d.key == key && &d.event == event)
    }

    #[test]
    fn displacement_on_third_insert() {
        // limit=2, sort by score ASC. Insert score 10, 20, then 5.
        let w = LiveWindow::new(sort_asc("score"), Some(2));

        let d1 = w.apply_mutation("a", Some(&rec("score", 10)), true);
        assert_eq!(events(&d1), vec![("a".to_string(), ChangeEventType::ENTER)]);

        let d2 = w.apply_mutation("b", Some(&rec("score", 20)), true);
        assert_eq!(events(&d2), vec![("b".to_string(), ChangeEventType::ENTER)]);

        // "c" at score 5 sorts before both → enters; "b" (score 20) is displaced.
        let d3 = w.apply_mutation("c", Some(&rec("score", 5)), true);
        assert!(has(&d3, "c", &ChangeEventType::ENTER));
        assert!(has(&d3, "b", &ChangeEventType::LEAVE));
        assert_eq!(d3.len(), 2);
    }

    #[test]
    fn promotion_after_in_window_leaves() {
        // Set up displacement: window holds [c(5), a(10)], b(20) retained out-of-window.
        let w = LiveWindow::new(sort_asc("score"), Some(2));
        w.apply_mutation("a", Some(&rec("score", 10)), true);
        w.apply_mutation("b", Some(&rec("score", 20)), true);
        w.apply_mutation("c", Some(&rec("score", 5)), true);

        // The in-window "c" leaves (delete). A slot frees → "b" (retained) is promoted.
        let d = w.apply_mutation("c", None, false);
        assert!(has(&d, "c", &ChangeEventType::LEAVE));
        assert!(has(&d, "b", &ChangeEventType::ENTER));
    }

    #[test]
    fn update_within_window() {
        let w = LiveWindow::new(sort_asc("score"), Some(3));
        w.apply_mutation("a", Some(&rec("score", 10)), true);
        w.apply_mutation("b", Some(&rec("score", 20)), true);

        // Update "a" to score 15 — still in top-3 → UPDATE only.
        let d = w.apply_mutation("a", Some(&rec("score", 15)), true);
        assert_eq!(events(&d), vec![("a".to_string(), ChangeEventType::UPDATE)]);
    }

    #[test]
    fn update_out_of_window() {
        // limit=2, window holds [a(10), b(20)], c(30) retained out-of-window.
        let w = LiveWindow::new(sort_asc("score"), Some(2));
        w.apply_mutation("a", Some(&rec("score", 10)), true);
        w.apply_mutation("b", Some(&rec("score", 20)), true);
        w.apply_mutation("c", Some(&rec("score", 30)), true); // retained, no in-window change

        // Update "a" to score 100 — pushed out of top-2; "c" (30) promoted in.
        let d = w.apply_mutation("a", Some(&rec("score", 100)), true);
        assert!(has(&d, "a", &ChangeEventType::LEAVE));
        assert!(has(&d, "c", &ChangeEventType::ENTER));
    }

    #[test]
    fn unbounded_no_displacement() {
        // limit=None → predicate-only, no displacement regardless of count.
        let w = LiveWindow::new(sort_asc("score"), None);

        let d1 = w.apply_mutation("a", Some(&rec("score", 10)), true);
        assert_eq!(events(&d1), vec![("a".to_string(), ChangeEventType::ENTER)]);
        let d2 = w.apply_mutation("b", Some(&rec("score", 20)), true);
        assert_eq!(events(&d2), vec![("b".to_string(), ChangeEventType::ENTER)]);
        let d3 = w.apply_mutation("c", Some(&rec("score", 5)), true);
        // Pure ENTER, no LEAVE displacement even though c sorts first.
        assert_eq!(events(&d3), vec![("c".to_string(), ChangeEventType::ENTER)]);

        // Update within → UPDATE.
        let d4 = w.apply_mutation("a", Some(&rec("score", 11)), true);
        assert_eq!(
            events(&d4),
            vec![("a".to_string(), ChangeEventType::UPDATE)]
        );

        // Predicate-false → LEAVE.
        let d5 = w.apply_mutation("b", Some(&rec("score", 20)), false);
        assert_eq!(events(&d5), vec![("b".to_string(), ChangeEventType::LEAVE)]);
    }

    /// Returns the value the window currently holds for `key`, if any.
    fn value_of(w: &LiveWindow, key: &str) -> Option<rmpv::Value> {
        let state = w.state.lock();
        state.iter().find(|r| r.key == key).map(|r| r.value.clone())
    }

    /// AC7b(iii): a cross-batch value-skew (a stale value read in one batch while
    /// a fresher value of the SAME key landed in another) is transient and
    /// self-heals — the NEXT per-key delta carries the LWW value and the window
    /// converges to it, emitting a single UPDATE the client keyed-map reconcile
    /// applies. We do NOT assert cross-batch atomicity (intentionally not
    /// provided); we assert convergence.
    #[test]
    fn cross_batch_value_skew_converges_on_next_delta() {
        let w = LiveWindow::new(sort_asc("score"), Some(3));

        // A full-scan batch seeds a STALE value for "k" (the skew: a concurrent
        // fresher write of the same key was observed in a different batch's
        // snapshot, so the result page carried the older value).
        let seed = w.apply_mutation("k", Some(&rec("score", 10)), true);
        assert!(has(&seed, "k", &ChangeEventType::ENTER));
        assert_eq!(value_of(&w, "k"), Some(rec("score", 10)));

        // The next per-key delta (the SyncEngine per-key UPDATE the client
        // reconcile consumes) carries the converged LWW value.
        let converge = w.apply_mutation("k", Some(&rec("score", 42)), true);
        assert_eq!(
            events(&converge),
            vec![("k".to_string(), ChangeEventType::UPDATE)]
        );
        assert_eq!(
            value_of(&w, "k"),
            Some(rec("score", 42)),
            "skewed value converges to the LWW value on the next per-key delta"
        );
        // The converging delta itself carries the LWW value (what the client
        // keyed-map writes), so no persistent wrong answer survives.
        let delta = converge.iter().find(|d| d.key == "k").unwrap();
        assert_eq!(delta.value, rec("score", 42));
    }

    /// A batch of a tracked key's removal and an untracked key's removal
    /// sends only the tracked key's LEAVE.
    #[test]
    fn a_batch_of_removals_leaves_only_the_tracked_key() {
        let w = LiveWindow::new(sort_asc("score"), Some(3));
        w.apply_mutation("a", Some(&rec("score", 1)), true);
        w.apply_mutation("b", Some(&rec("score", 2)), true);
        let deltas = w.apply_batch([("a", None, false), ("z", None, false)]);
        assert_eq!(events(&deltas), vec![("a".to_string(), ChangeEventType::LEAVE)]);
    }

    /// The tracked rows in window order.
    fn rows_of(w: &LiveWindow) -> Vec<(String, rmpv::Value)> {
        w.state
            .lock()
            .iter()
            .map(|r| (r.key.clone(), r.value.clone()))
            .collect()
    }

    /// The rows a subscriber holds after applying `deltas` to `held`.
    fn replay(
        mut held: std::collections::BTreeMap<String, rmpv::Value>,
        deltas: Vec<WindowDelta>,
    ) -> std::collections::BTreeMap<String, rmpv::Value> {
        for delta in deltas {
            match delta.event {
                ChangeEventType::ENTER | ChangeEventType::UPDATE => {
                    held.insert(delta.key, delta.value);
                }
                ChangeEventType::LEAVE => {
                    held.remove(&delta.key);
                }
            }
        }
        held
    }

    fn in_window(w: &LiveWindow) -> std::collections::BTreeMap<String, rmpv::Value> {
        let take = w.limit.map_or(usize::MAX, |l| l as usize);
        rows_of(w).into_iter().take(take).collect()
    }

    proptest::proptest! {
        /// A batch leaves the window's rows, and the rows its deltas leave a
        /// subscriber holding, exactly as the same mutations applied one by
        /// one, each a batch of one; checked against the per-row algorithm
        /// this replaced before it was removed. Ten keys and a limit of at least five keep the rows within
        /// the trim of twice the limit, where the two paths can differ.
        #[test]
        fn a_batch_ends_where_its_mutations_one_by_one_end(
            limit in proptest::option::of(5u32..9),
            seed in proptest::collection::vec((0u8..10, 0i64..20), 0..10),
            batch in proptest::collection::vec(
                (0u8..10, proptest::option::of(0i64..20), proptest::bool::ANY),
                0..24,
            ),
        ) {
            let one_by_one = LiveWindow::new(sort_asc("score"), limit);
            let batched = LiveWindow::new(sort_asc("score"), limit);
            for (k, n) in &seed {
                let key = format!("k{k}");
                one_by_one.apply_mutation(&key, Some(&rec("score", *n)), true);
                batched.apply_mutation(&key, Some(&rec("score", *n)), true);
            }
            let held = in_window(&one_by_one);
            let mutations: Vec<(String, Option<rmpv::Value>, bool)> = batch
                .iter()
                .map(|(k, n, m)| (format!("k{k}"), n.map(|n| rec("score", n)), *m))
                .collect();
            let mut held_one_by_one = held.clone();
            for (key, value, matches) in &mutations {
                held_one_by_one = replay(
                    held_one_by_one,
                    one_by_one.apply_mutation(key, value.as_ref(), *matches),
                );
            }
            let held_batched = replay(
                held,
                batched.apply_batch(mutations.iter().map(|(k, v, m)| (k.as_str(), v.as_ref(), *m))),
            );
            proptest::prop_assert_eq!(rows_of(&batched), rows_of(&one_by_one));
            proptest::prop_assert_eq!(&held_batched, &held_one_by_one);
            proptest::prop_assert_eq!(held_batched, in_window(&batched));
        }

        /// Past the trim of twice the limit, a batch over a window loaded
        /// within it shows the page a fresh sort and limit over the values its
        /// keys end with gives, and so do the rows its deltas leave a
        /// subscriber holding.
        #[test]
        fn a_batch_shows_the_page_of_the_values_its_keys_end_with(
            limit in 1u32..5,
            seed in proptest::collection::vec(0i64..20, 0..10),
            batch in proptest::collection::vec(
                (0u8..20, proptest::option::of(0i64..20), proptest::bool::ANY),
                0..40,
            ),
        ) {
            let w = LiveWindow::new(sort_asc("score"), Some(limit));
            let seed: Vec<(String, rmpv::Value)> = seed
                .into_iter()
                .take(2 * limit as usize)
                .enumerate()
                .map(|(i, n)| (format!("k{i}"), rec("score", n)))
                .collect();
            let _ = w.apply_batch(seed.iter().map(|(k, v)| (k.as_str(), Some(v), true)));
            let mut end: std::collections::BTreeMap<String, rmpv::Value> = seed.into_iter().collect();
            let batch: Vec<(String, Option<rmpv::Value>)> = batch
                .iter()
                .map(|(k, n, m)| (format!("k{k}"), n.filter(|_| *m).map(|n| rec("score", n))))
                .collect();
            for (key, value) in &batch {
                match value {
                    Some(value) => end.insert(key.clone(), value.clone()),
                    None => end.remove(key),
                };
            }
            let held = replay(
                in_window(&w),
                w.apply_batch(batch.iter().map(|(k, v)| (k.as_str(), v.as_ref(), v.is_some()))),
            );
            let mut page: Vec<(i64, String)> = end
                .iter()
                .map(|(k, v)| (v.as_map().expect("a map")[0].1.as_i64().expect("an int"), k.clone()))
                .collect();
            page.sort();
            page.truncate(limit as usize);
            let page: std::collections::BTreeMap<String, rmpv::Value> =
                page.into_iter().map(|(n, k)| (k, rec("score", n))).collect();
            proptest::prop_assert_eq!(&in_window(&w), &page);
            proptest::prop_assert_eq!(held, page);
        }
    }

    /// Report-only: the time to load a 10 000-row window from a sorted scan,
    /// one row at a time and in one batch, five runs each.
    #[test]
    #[ignore = "measurement; run with --ignored --nocapture"]
    fn initial_load_of_10000_rows() {
        let rows: Vec<(String, rmpv::Value)> = (0..10_000)
            .map(|n| (format!("r{n:05}"), rec("v", n)))
            .collect();
        let report = |name: &str, mut runs: Vec<std::time::Duration>| {
            for (i, run) in runs.iter().enumerate() {
                println!("{name} run {i}: {:.3} ms", run.as_secs_f64() * 1e3);
            }
            runs.sort();
            let median = runs[runs.len() / 2].as_secs_f64() * 1e3;
            println!("{name} median: {median:.3} ms");
        };
        let (mut per_row, mut batched) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            let w = LiveWindow::new(sort_asc("v"), Some(10_000));
            let start = std::time::Instant::now();
            for (key, value) in &rows {
                let _ = w.apply_mutation(key, Some(value), true);
            }
            per_row.push(start.elapsed());
            let w = LiveWindow::new(sort_asc("v"), Some(10_000));
            let start = std::time::Instant::now();
            let _ = w.apply_batch(rows.iter().map(|(k, v)| (k.as_str(), Some(v), true)));
            batched.push(start.elapsed());
        }
        report("initial_load per_row", per_row);
        report("initial_load batch", batched);

        // A write that does not match the query, to a key the window does not
        // track, against the loaded window: per call, over 1000 calls.
        let w = LiveWindow::new(sort_asc("v"), Some(10_000));
        let _ = w.apply_batch(rows.iter().map(|(k, v)| (k.as_str(), Some(v), true)));
        let mut non_matching = Vec::new();
        for _ in 0..5 {
            let start = std::time::Instant::now();
            for _ in 0..1000 {
                assert!(w.apply_mutation("absent", None, false).is_empty());
            }
            non_matching.push(start.elapsed() / 1000);
        }
        report("non_matching_write per call", non_matching);
    }

    // ---- HLC monotone guard tests ----

    #[test]
    fn hlc_guard_accepts_strictly_greater_hlc() {
        let w = LiveWindow::new(sort_asc("score"), None);

        // First write at millis=100, counter=0 — should be accepted.
        let d1 = w.apply_mutation_with_hlc("k", Some(&rec("score", 1)), true, 100, 0);
        assert!(has(&d1, "k", &ChangeEventType::ENTER));

        // Second write at millis=101, counter=0 — strictly greater millis, accepted.
        let d2 = w.apply_mutation_with_hlc("k", Some(&rec("score", 2)), true, 101, 0);
        assert!(has(&d2, "k", &ChangeEventType::UPDATE));

        // Same millis, greater counter (101, 1) — strictly greater, accepted.
        let d3 = w.apply_mutation_with_hlc("k", Some(&rec("score", 3)), true, 101, 1);
        assert!(has(&d3, "k", &ChangeEventType::UPDATE));
    }

    #[test]
    fn hlc_guard_drops_equal_hlc() {
        let w = LiveWindow::new(sort_asc("score"), None);

        // Accept an initial write.
        let d1 = w.apply_mutation_with_hlc("k", Some(&rec("score", 10)), true, 200, 5);
        assert!(has(&d1, "k", &ChangeEventType::ENTER));

        // Repeat the exact same HLC — must be silently dropped (empty delta).
        let d2 = w.apply_mutation_with_hlc("k", Some(&rec("score", 99)), true, 200, 5);
        assert!(d2.is_empty(), "equal HLC must be silently dropped");
    }

    #[test]
    fn hlc_guard_drops_stale_hlc() {
        let w = LiveWindow::new(sort_asc("score"), None);

        // Accept a write at millis=500, counter=0.
        let d1 = w.apply_mutation_with_hlc("k", Some(&rec("score", 10)), true, 500, 0);
        assert!(has(&d1, "k", &ChangeEventType::ENTER));

        // A stale datastore page arrives at millis=100 — must be silently dropped.
        let d2 = w.apply_mutation_with_hlc("k", Some(&rec("score", 99)), true, 100, 0);
        assert!(
            d2.is_empty(),
            "stale HLC (older millis) must not overwrite newer in-memory value"
        );

        // A stale write at same millis but lower counter — also dropped.
        let d3 = w.apply_mutation_with_hlc("k", Some(&rec("score", 77)), true, 500, 0);
        assert!(
            d3.is_empty(),
            "equal HLC is treated as stale (not strictly greater)"
        );
    }

    #[test]
    fn hlc_guard_is_per_key_independent() {
        let w = LiveWindow::new(sort_asc("score"), None);

        // Write key "a" at HLC (100, 0), key "b" at HLC (50, 0).
        w.apply_mutation_with_hlc("a", Some(&rec("score", 1)), true, 100, 0);
        w.apply_mutation_with_hlc("b", Some(&rec("score", 2)), true, 50, 0);

        // A stale update for "a" at HLC (90, 0) must be dropped.
        let stale_a = w.apply_mutation_with_hlc("a", Some(&rec("score", 99)), true, 90, 0);
        assert!(
            stale_a.is_empty(),
            "stale write for key 'a' must be dropped"
        );

        // A fresh update for "b" at HLC (60, 0) must be accepted (b's last-seen is 50).
        let fresh_b = w.apply_mutation_with_hlc("b", Some(&rec("score", 3)), true, 60, 0);
        assert!(
            has(&fresh_b, "b", &ChangeEventType::UPDATE),
            "fresh write for key 'b' must be accepted"
        );
    }

    #[test]
    fn plain_apply_mutation_bypasses_hlc_guard() {
        let w = LiveWindow::new(sort_asc("score"), None);

        // Seed via the HLC-guarded path at HLC (1000, 0).
        w.apply_mutation_with_hlc("k", Some(&rec("score", 10)), true, 1000, 0);

        // The plain path (live CRDT write, no HLC) always passes through regardless
        // of what the HLC map holds. This ensures live mutations are never silently
        // swallowed by a stale guard entry.
        let d = w.apply_mutation("k", Some(&rec("score", 20)), true);
        assert!(
            has(&d, "k", &ChangeEventType::UPDATE),
            "plain apply_mutation must bypass the HLC guard"
        );
    }
}
