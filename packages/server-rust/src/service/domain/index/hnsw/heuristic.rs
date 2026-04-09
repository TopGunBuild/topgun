use std::collections::BTreeMap;

use ordered_float::OrderedFloat;

use crate::service::domain::index::hnsw::graph::UndirectedGraph;
use crate::service::domain::index::hnsw::types::{ElementId, Heuristic};

// ---------------------------------------------------------------------------
// DoublePriorityQueue
// ---------------------------------------------------------------------------

/// Min/max priority queue for beam-search working sets.
///
/// BTreeMap is chosen over a binary heap because we need efficient access to
/// both the minimum (nearest candidate) and maximum (furthest result) without
/// a second data structure.  Tie-breaking on equal distances stores multiple
/// elements per key using a Vec.
pub struct DoublePriorityQueue {
    map: BTreeMap<OrderedFloat<f64>, Vec<ElementId>>,
    len: usize,
}

impl DoublePriorityQueue {
    #[must_use]
    pub fn new() -> Self {
        DoublePriorityQueue {
            map: BTreeMap::new(),
            len: 0,
        }
    }

    /// Insert an element with its distance.
    pub fn insert(&mut self, id: ElementId, distance: f64) {
        self.map
            .entry(OrderedFloat(distance))
            .or_default()
            .push(id);
        self.len += 1;
    }

    /// Remove and return the nearest (minimum-distance) element.
    pub fn pop_nearest(&mut self) -> Option<(ElementId, f64)> {
        let key = *self.map.keys().next()?;
        let bucket = self.map.get_mut(&key)?;
        let id = bucket.pop()?;
        self.len -= 1;
        if bucket.is_empty() {
            self.map.remove(&key);
        }
        Some((id, key.0))
    }

    /// Peek at the nearest (minimum-distance) element without removing it.
    #[must_use]
    pub fn peek_nearest(&self) -> Option<(ElementId, f64)> {
        self.map
            .iter()
            .next()
            .and_then(|(k, v)| v.last().map(|&id| (id, k.0)))
    }

    /// Peek at the furthest (maximum-distance) element without removing it.
    #[must_use]
    pub fn peek_furthest(&self) -> Option<(ElementId, f64)> {
        self.map
            .iter()
            .next_back()
            .and_then(|(k, v)| v.last().map(|&id| (id, k.0)))
    }

    /// Remove and return the furthest (maximum-distance) element.
    pub fn pop_furthest(&mut self) -> Option<(ElementId, f64)> {
        let key = *self.map.keys().next_back()?;
        let bucket = self.map.get_mut(&key)?;
        let id = bucket.pop()?;
        self.len -= 1;
        if bucket.is_empty() {
            self.map.remove(&key);
        }
        Some((id, key.0))
    }

    /// Number of elements in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` when the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for DoublePriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// select_neighbors — HNSW paper Algorithm 4
// ---------------------------------------------------------------------------

/// Select at most `m` neighbors from `candidates` using the requested heuristic.
///
/// `candidates` is a slice of (ElementId, distance-to-query) pairs.
/// The heuristic controls whether the candidate set is expanded and whether
/// pruned candidates fill remaining slots.
#[allow(clippy::too_many_arguments)]
pub fn select_neighbors(
    candidates: &[(ElementId, f64)],
    m: usize,
    heuristic: Heuristic,
    graph: &UndirectedGraph,
    dist_fn: &dyn Fn(ElementId, ElementId) -> f64,
) -> Vec<ElementId> {
    let extend = matches!(heuristic, Heuristic::Extended | Heuristic::ExtendedAndKeep);
    let keep_pruned = matches!(heuristic, Heuristic::KeepPruned | Heuristic::ExtendedAndKeep);

    // Build working candidate list, optionally expanded with neighbors-of-candidates.
    let mut working: Vec<(ElementId, f64)> = candidates.to_vec();

    if extend {
        // Expand: add neighbors of every candidate that aren't already present.
        let existing_ids: std::collections::HashSet<ElementId> =
            working.iter().map(|(id, _)| *id).collect();
        let mut extra: Vec<(ElementId, f64)> = Vec::new();
        for &(cand_id, _) in candidates {
            if let Some(nbrs) = graph.neighbors(&cand_id) {
                for nbr in nbrs.iter() {
                    if !existing_ids.contains(&nbr) {
                        let d = dist_fn(nbr, cand_id); // distance from nbr to "query proxy"
                        extra.push((nbr, d));
                    }
                }
            }
        }
        working.extend(extra);
        // Deduplicate by id — keep the entry with the smallest distance.
        working.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap()));
        working.dedup_by(|a, b| {
            if a.0 == b.0 {
                if a.1 < b.1 {
                    b.1 = a.1;
                }
                true
            } else {
                false
            }
        });
    }

    // Sort ascending by distance.
    working.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut result: Vec<ElementId> = Vec::with_capacity(m);
    let mut pruned: Vec<ElementId> = Vec::new();

    for &(cand_id, cand_dist) in &working {
        if result.len() >= m {
            break;
        }
        // Keep the candidate if it is closer to the query than to every already-selected neighbor.
        // This ensures the resulting graph remains navigable by preferring diverse connections.
        let closer_to_query = result.iter().all(|&selected| {
            let d_cand_to_selected = dist_fn(cand_id, selected);
            cand_dist < d_cand_to_selected
        });
        if closer_to_query || result.is_empty() {
            result.push(cand_id);
        } else {
            pruned.push(cand_id);
        }
    }

    if keep_pruned && result.len() < m {
        for id in pruned {
            if result.len() >= m {
                break;
            }
            result.push(id);
        }
    }

    result.truncate(m);
    result
}
