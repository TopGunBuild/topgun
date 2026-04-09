use std::collections::HashMap;

use crate::service::domain::index::hnsw::types::{DynamicSet, ElementId};

/// Undirected adjacency-list graph used by each HNSW layer.
///
/// Each node owns a `Box<dyn DynamicSet>` that tracks its neighbors.
/// Edges are always inserted and removed bidirectionally so that graph
/// traversal from any entry point remains consistent.
pub struct UndirectedGraph {
    nodes: HashMap<ElementId, Box<dyn DynamicSet>>,
}

impl UndirectedGraph {
    #[must_use]
    pub fn new() -> Self {
        UndirectedGraph {
            nodes: HashMap::new(),
        }
    }

    /// Register a node with an (initially empty) neighbor set.
    ///
    /// The set type is supplied by the caller so that each layer can use
    /// the capacity appropriate for its level (m vs m0).
    pub fn add_node(&mut self, id: ElementId, set: Box<dyn DynamicSet>) {
        self.nodes.entry(id).or_insert(set);
    }

    /// Insert edge a↔b into both nodes' neighbor sets.
    pub fn add_edge(&mut self, a: ElementId, b: ElementId) {
        if let Some(set) = self.nodes.get_mut(&a) {
            set.insert(b);
        }
        if let Some(set) = self.nodes.get_mut(&b) {
            set.insert(a);
        }
    }

    /// Remove edge a↔b from both nodes' neighbor sets.
    pub fn remove_edge(&mut self, a: ElementId, b: ElementId) {
        if let Some(set) = self.nodes.get_mut(&a) {
            set.remove(&b);
        }
        if let Some(set) = self.nodes.get_mut(&b) {
            set.remove(&a);
        }
    }

    /// Remove a node and all back-edges pointing to it from its neighbors.
    ///
    /// The back-edge targets are collected before the node is removed to
    /// avoid holding a mutable borrow on `self.nodes` while iterating.
    pub fn remove_node(&mut self, id: ElementId) {
        let neighbors: Vec<ElementId> = self
            .nodes
            .get(&id)
            .map(|set| set.iter().collect())
            .unwrap_or_default();
        self.nodes.remove(&id);
        for neighbor in neighbors {
            if let Some(set) = self.nodes.get_mut(&neighbor) {
                set.remove(&id);
            }
        }
    }

    /// Returns a reference to the neighbor set of `id`, if present.
    #[must_use]
    pub fn neighbors(&self, id: &ElementId) -> Option<&dyn DynamicSet> {
        self.nodes.get(id).map(std::convert::AsRef::as_ref)
    }

    /// Returns a mutable reference to the neighbor set of `id`, if present.
    /// Returns a mutable reference to the boxed neighbor set for direct manipulation.
    pub fn neighbors_mut_box(&mut self, id: &ElementId) -> Option<&mut Box<dyn DynamicSet>> {
        self.nodes.get_mut(id)
    }

    /// Returns `true` if the node is registered in this graph.
    #[must_use]
    pub fn has_node(&self, id: &ElementId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Returns the total number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Iterate over all node IDs.
    pub fn node_ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.nodes.keys().copied()
    }
}

impl Default for UndirectedGraph {
    fn default() -> Self {
        Self::new()
    }
}
