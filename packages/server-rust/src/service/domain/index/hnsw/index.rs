use std::collections::{HashMap, HashSet};

use topgun_core::vector::{Distance, SharedVector, distance_for_metric};

use crate::service::domain::index::hnsw::heuristic::{DoublePriorityQueue, select_neighbors};
use crate::service::domain::index::hnsw::layer::{Layer, random_level};
use crate::service::domain::index::hnsw::types::{ElementId, Heuristic, HnswFlavor, HnswParams};

/// In-memory HNSW approximate nearest-neighbor index.
///
/// No thread safety — callers must synchronize externally.
/// No persistence — all state lives in heap memory.
/// No integration with `IndexRegistry` — standalone component.
pub struct Hnsw {
    params: HnswParams,
    flavor: HnswFlavor,
    layers: Vec<Layer>,
    entry_point: Option<ElementId>,
    vectors: HashMap<ElementId, SharedVector>,
    deleted: HashSet<ElementId>,
    dist: Box<dyn Distance>,
    count: usize,
}

impl Hnsw {
    /// Construct an empty index with the given parameters.
    #[must_use]
    pub fn new(params: HnswParams) -> Self {
        let flavor = match params.m {
            8 => HnswFlavor::M8,
            12 => HnswFlavor::M12,
            16 => HnswFlavor::M16,
            _ => HnswFlavor::Custom {
                m: params.m,
                m0: params.m0,
            },
        };
        let dist = distance_for_metric(params.distance);
        Hnsw {
            params,
            flavor,
            layers: Vec::new(),
            entry_point: None,
            vectors: HashMap::new(),
            deleted: HashSet::new(),
            dist,
            count: 0,
        }
    }

    /// Number of non-deleted elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` when there are no non-deleted elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns `true` if `id` is present and not deleted.
    #[must_use]
    pub fn contains(&self, id: &ElementId) -> bool {
        self.vectors.contains_key(id) && !self.deleted.contains(id)
    }

    // -----------------------------------------------------------------------
    // Distance helpers
    // -----------------------------------------------------------------------

    fn compute_distance(&self, a: &SharedVector, b: &SharedVector) -> f64 {
        let av = a.vector().to_f32_vec();
        let bv = b.vector().to_f32_vec();
        self.dist.compute(&av, &bv)
    }

    fn distance_to_query(&self, id: ElementId, query: &[f32]) -> f64 {
        if let Some(v) = self.vectors.get(&id) {
            let vf = v.vector().to_f32_vec();
            self.dist.compute(&vf, query)
        } else {
            f64::MAX
        }
    }

    fn dist_between_elements(&self, a: ElementId, b: ElementId) -> f64 {
        match (self.vectors.get(&a), self.vectors.get(&b)) {
            (Some(va), Some(vb)) => self.compute_distance(va, vb),
            _ => f64::MAX,
        }
    }

    // -----------------------------------------------------------------------
    // search_layer — Algorithm 2
    // -----------------------------------------------------------------------

    /// Beam search within a single layer.
    ///
    /// `entry_points` are the starting candidates. `ef` controls the beam width.
    /// Returns up to `ef` nearest elements as (id, distance) pairs.
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[ElementId],
        ef: usize,
        layer_idx: usize,
    ) -> Vec<(ElementId, f64)> {
        let Some(layer) = self.layers.get(layer_idx) else {
            return Vec::new();
        };

        let mut candidates = DoublePriorityQueue::new();
        let mut results = DoublePriorityQueue::new();
        let mut visited: HashSet<ElementId> = HashSet::new();

        for &ep in entry_points {
            let d = self.distance_to_query(ep, query);
            candidates.insert(ep, d);
            results.insert(ep, d);
            visited.insert(ep);
        }

        while let Some((cand_id, cand_dist)) = candidates.pop_nearest() {
            let furthest_result_dist = results
                .peek_furthest()
                .map_or(f64::MAX, |(_, d)| d);

            if cand_dist > furthest_result_dist {
                break;
            }

            if let Some(nbrs) = layer.graph.neighbors(&cand_id) {
                let nbr_ids: Vec<ElementId> = nbrs.iter().collect();
                for nbr in nbr_ids {
                    if visited.contains(&nbr) || self.deleted.contains(&nbr) {
                        continue;
                    }
                    visited.insert(nbr);
                    let d = self.distance_to_query(nbr, query);
                    let furthest = results
                        .peek_furthest()
                        .map_or(f64::MAX, |(_, fd)| fd);
                    if d < furthest || results.len() < ef {
                        candidates.insert(nbr, d);
                        results.insert(nbr, d);
                        if results.len() > ef {
                            results.pop_furthest();
                        }
                    }
                }
            }
        }

        // Drain results into a Vec sorted by ascending distance.
        let mut out: Vec<(ElementId, f64)> = Vec::with_capacity(results.len());
        while let Some(item) = results.pop_nearest() {
            out.push(item);
        }
        out
    }

    // -----------------------------------------------------------------------
    // insert — Algorithm 1
    // -----------------------------------------------------------------------

    /// Insert a vector into the index.
    ///
    /// # Panics
    ///
    /// Panics if the vector dimension does not match `params.dimension`.
    pub fn insert(&mut self, id: ElementId, vector: SharedVector) {
        assert_eq!(
            vector.dimension(),
            self.params.dimension as usize,
            "dimension mismatch: expected {}, got {}",
            self.params.dimension,
            vector.dimension()
        );

        // If re-inserting a previously deleted element, un-delete it.
        self.deleted.remove(&id);
        self.vectors.insert(id, vector);
        self.count += 1;

        let level = random_level(self.params.ml);

        // Ensure we have enough layer slots.
        while self.layers.len() <= level {
            self.layers.push(Layer::new());
        }

        // Register the new node in every layer up to its assigned level.
        for l in 0..=level {
            let is_base = l == 0;
            let set = self.flavor.create_set(is_base);
            self.layers[l].insert_node(id, set);
        }

        // If this is the first node, it becomes the entry point.
        let Some(ep) = self.entry_point else {
            self.entry_point = Some(id);
            return;
        };

        let current_max_level = self.layers.len().saturating_sub(1);
        let query = self.vectors[&id].vector().to_f32_vec();

        // Greedy walk from the top layer down to level+1 to find the best
        // entry point for the insertion layers.
        let mut current_ep = vec![ep];
        for lc in (level + 1..=current_max_level).rev() {
            let found = self.search_layer(&query, &current_ep, 1, lc);
            if let Some((best, _)) = found.into_iter().next() {
                current_ep = vec![best];
            }
        }

        let heuristic = match (self.params.extend_candidates, self.params.keep_pruned_connections) {
            (true, true) => Heuristic::ExtendedAndKeep,
            (true, false) => Heuristic::Extended,
            (false, true) => Heuristic::KeepPruned,
            (false, false) => Heuristic::Standard,
        };

        for lc in (0..=level.min(current_max_level)).rev() {
            let ef = self.params.ef_construction as usize;
            let candidates = self.search_layer(&query, &current_ep, ef, lc);

            // Use search results as the best entry points for the next lower layer.
            current_ep = candidates.iter().map(|(cid, _)| *cid).collect();

            let m = if lc == 0 {
                self.params.m0 as usize
            } else {
                self.params.m as usize
            };

            // Select neighbors using the configured heuristic.
            let neighbors = {
                let graph = &self.layers[lc].graph;
                let dist_fn = |a: ElementId, b: ElementId| self.dist_between_elements(a, b);
                select_neighbors(&candidates, m, heuristic, graph, &dist_fn)
            };

            // Wire bidirectional edges to selected neighbors.
            for &nbr in &neighbors {
                self.layers[lc].graph.add_edge(id, nbr);
            }

            // Prune oversized neighbor lists for existing nodes.
            let overfull: Vec<ElementId> = neighbors
                .iter()
                .filter_map(|&nbr| {
                    self.layers[lc]
                        .graph
                        .neighbors(&nbr)
                        .map(|s| (nbr, s.len(), s.capacity()))
                })
                .filter(|(_, len, cap)| len > cap)
                .map(|(nbr, _, _)| nbr)
                .collect();

            for nbr in overfull {
                self.prune_neighbors(nbr, lc, m, heuristic);
            }
        }

        // Update entry point if the new node reaches a higher layer.
        if level > current_max_level {
            self.entry_point = Some(id);
        }
    }

    /// Prune neighbor list of `node` in `layer_idx` to at most `m` edges.
    fn prune_neighbors(
        &mut self,
        node: ElementId,
        layer_idx: usize,
        m: usize,
        heuristic: Heuristic,
    ) {
        let current_neighbors: Vec<(ElementId, f64)> = self.layers[layer_idx]
            .graph
            .neighbors(&node)
            .map(|s| {
                s.iter()
                    .map(|nbr| (nbr, self.dist_between_elements(node, nbr)))
                    .collect()
            })
            .unwrap_or_default();

        if current_neighbors.len() <= m {
            return;
        }

        let selected = {
            let graph = &self.layers[layer_idx].graph;
            let dist_fn = |a: ElementId, b: ElementId| self.dist_between_elements(a, b);
            select_neighbors(&current_neighbors, m, heuristic, graph, &dist_fn)
        };

        let selected_set: HashSet<ElementId> = selected.into_iter().collect();

        // Remove edges to neighbors not in the selected set.
        let to_remove: Vec<ElementId> = current_neighbors
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !selected_set.contains(id))
            .collect();

        for nbr in to_remove {
            self.layers[layer_idx].graph.remove_edge(node, nbr);
        }
    }

    // -----------------------------------------------------------------------
    // search — public API
    // -----------------------------------------------------------------------

    /// Search for the `k` nearest neighbors of `query`.
    ///
    /// Returns up to `k` (id, distance) pairs sorted by ascending distance.
    /// Deleted elements are excluded from results.
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<(ElementId, f64)> {
        let Some(ep) = self.entry_point else {
            return Vec::new();
        };

        let max_layer = self.layers.len().saturating_sub(1);
        let mut current_ep = vec![ep];

        // Greedy walk from top to layer 1 with ef=1.
        for lc in (1..=max_layer).rev() {
            let found = self.search_layer(query, &current_ep, 1, lc);
            if let Some((best, _)) = found.into_iter().next() {
                current_ep = vec![best];
            }
        }

        // Full search at layer 0 with the requested ef.
        let mut results = self.search_layer(query, &current_ep, ef.max(k), 0);

        // Exclude soft-deleted elements.
        results.retain(|(id, _)| !self.deleted.contains(id));

        results.truncate(k);
        results
    }

    // -----------------------------------------------------------------------
    // remove
    // -----------------------------------------------------------------------

    /// Soft-delete an element from the index.
    ///
    /// The element is excluded from future search results but its vectors and
    /// graph edges remain until `optimize()` is called. This avoids the
    /// expensive re-wiring that would be needed for eager removal.
    pub fn remove(&mut self, id: ElementId) -> bool {
        if self.vectors.contains_key(&id) && !self.deleted.contains(&id) {
            self.deleted.insert(id);
            self.count -= 1;
            true
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // optimize
    // -----------------------------------------------------------------------

    /// Rebuild the index from scratch, discarding all deleted elements.
    ///
    /// Collect all non-deleted id→vector pairs, reset internal state, then
    /// re-insert each element. This is O(n log n) and may take seconds on
    /// large indexes — intended for periodic maintenance, not the hot path.
    pub fn optimize(&mut self) {
        let live: Vec<(ElementId, SharedVector)> = self
            .vectors
            .iter()
            .filter(|(id, _)| !self.deleted.contains(id))
            .map(|(id, v)| (*id, v.clone()))
            .collect();

        // Reset all mutable state while keeping params and dist.
        self.layers.clear();
        self.entry_point = None;
        self.vectors.clear();
        self.deleted.clear();
        self.count = 0;

        for (id, vector) in live {
            self.insert(id, vector);
        }
    }
}
