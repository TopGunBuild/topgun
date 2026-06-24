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
        }
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
    fn row_before(&self, a: &WindowRow, b: &WindowRow) -> bool {
        let a_cursor = self.row_cursor(&a.key, &a.value);
        is_after_cursor(&b.key, &b.value, &a_cursor)
    }

    /// Sorts the tracked rows in place by the keyset comparator (ascending window order).
    fn sort_state(&self, state: &mut [WindowRow]) {
        state.sort_by(|a, b| {
            if self.row_before(a, b) {
                std::cmp::Ordering::Less
            } else if self.row_before(b, a) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
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
        match self.limit {
            None => self.apply_unbounded(key, new_value, matches_predicate),
            Some(limit) => self.apply_bounded(key, new_value, matches_predicate, limit as usize),
        }
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

    /// Unbounded window: predicate-only membership, no displacement and no retention of
    /// out-of-window rows (there is no "out of window" when the window is unbounded).
    fn apply_unbounded(
        &self,
        key: &str,
        new_value: Option<&rmpv::Value>,
        matches_predicate: bool,
    ) -> Vec<WindowDelta> {
        let mut state = self.state.lock();
        let existing = state.iter().position(|r| r.key == key);
        let stays = matches_predicate && new_value.is_some();

        match (existing, stays) {
            (None, false) => Vec::new(),
            (None, true) => {
                let value = new_value.cloned().unwrap_or(rmpv::Value::Nil);
                state.push(WindowRow {
                    key: key.to_string(),
                    value: value.clone(),
                });
                self.sort_state(&mut state);
                vec![WindowDelta {
                    key: key.to_string(),
                    value,
                    event: ChangeEventType::ENTER,
                }]
            }
            (Some(idx), true) => {
                let value = new_value.cloned().unwrap_or(rmpv::Value::Nil);
                state[idx].value = value.clone();
                self.sort_state(&mut state);
                vec![WindowDelta {
                    key: key.to_string(),
                    value,
                    event: ChangeEventType::UPDATE,
                }]
            }
            (Some(idx), false) => {
                state.remove(idx);
                vec![WindowDelta {
                    key: key.to_string(),
                    value: rmpv::Value::Nil,
                    event: ChangeEventType::LEAVE,
                }]
            }
        }
    }

    /// Bounded top-N window with displacement and promotion.
    fn apply_bounded(
        &self,
        key: &str,
        new_value: Option<&rmpv::Value>,
        matches_predicate: bool,
        limit: usize,
    ) -> Vec<WindowDelta> {
        let mut state = self.state.lock();

        // Snapshot the keys currently in-window (first `limit` by sort order) so we can
        // diff membership after mutating + re-sorting.
        let in_window_before: std::collections::HashSet<String> =
            state.iter().take(limit).map(|r| r.key.clone()).collect();

        let existing = state.iter().position(|r| r.key == key);
        let stays = matches_predicate && new_value.is_some();

        match (existing, stays) {
            (None, false) => return Vec::new(),
            (None, true) => {
                let value = new_value.cloned().unwrap_or(rmpv::Value::Nil);
                state.push(WindowRow {
                    key: key.to_string(),
                    value,
                });
            }
            (Some(idx), true) => {
                state[idx].value = new_value.cloned().unwrap_or(rmpv::Value::Nil);
            }
            (Some(idx), false) => {
                // Delete or predicate-false: drop the row entirely (not a retained
                // candidate — it no longer matches the query).
                state.remove(idx);
            }
        }

        self.sort_state(&mut state);

        // Re-derive in-window membership after the mutation.
        let in_window_after: std::collections::HashSet<String> =
            state.iter().take(limit).map(|r| r.key.clone()).collect();

        let mut deltas = Vec::new();

        // Rows that left the top-N (displacement or removal) → LEAVE.
        for k in &in_window_before {
            if !in_window_after.contains(k) {
                deltas.push(WindowDelta {
                    key: k.clone(),
                    value: rmpv::Value::Nil,
                    event: ChangeEventType::LEAVE,
                });
            }
        }

        // Rows that entered the top-N (the new row, or a promoted retained row) → ENTER.
        for row in state.iter().take(limit) {
            if !in_window_before.contains(&row.key) {
                deltas.push(WindowDelta {
                    key: row.key.clone(),
                    value: row.value.clone(),
                    event: ChangeEventType::ENTER,
                });
            }
        }

        // A row that stayed in-window and was the mutated key → UPDATE.
        if matches_predicate
            && new_value.is_some()
            && in_window_before.contains(key)
            && in_window_after.contains(key)
        {
            let value = new_value.cloned().unwrap_or(rmpv::Value::Nil);
            deltas.push(WindowDelta {
                key: key.to_string(),
                value,
                event: ChangeEventType::UPDATE,
            });
        }

        // Trim retained out-of-window candidates: we only need to retain enough to refill
        // freed slots after later removals. Retaining everything seen would grow unbounded,
        // so cap retained rows at `limit` beyond the window (a displaced row plus headroom).
        let retain_cap = limit.saturating_mul(2);
        if state.len() > retain_cap {
            state.truncate(retain_cap);
        }

        deltas
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
        assert!(stale_a.is_empty(), "stale write for key 'a' must be dropped");

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
