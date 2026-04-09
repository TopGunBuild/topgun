use rand::Rng;

use crate::service::domain::index::hnsw::graph::UndirectedGraph;
use crate::service::domain::index::hnsw::types::{DynamicSet, ElementId};

/// One layer of the HNSW multi-layer graph.
///
/// Layer 0 is the base (densest) layer; higher layers grow sparser.
/// Each layer maintains an independent `UndirectedGraph` so that beam
/// search can be restricted to a single layer without filtering.
pub struct Layer {
    pub graph: UndirectedGraph,
}

impl Layer {
    #[must_use]
    pub fn new() -> Self {
        Layer {
            graph: UndirectedGraph::new(),
        }
    }

    /// Register a node in this layer's graph.
    pub fn insert_node(&mut self, id: ElementId, set: Box<dyn DynamicSet>) {
        self.graph.add_node(id, set);
    }

    /// Remove a node and all its back-edges from this layer.
    pub fn remove_node(&mut self, id: ElementId) {
        self.graph.remove_node(id);
    }

    /// Returns `true` if the node is present in this layer.
    #[must_use]
    pub fn has_node(&self, id: &ElementId) -> bool {
        self.graph.has_node(id)
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

/// Assign a random level to a new element using the HNSW paper's formula.
///
/// Formula: `floor(-ln(uniform(0,1)) * ml)` where `ml = 1/ln(m)`.
/// This gives an exponentially decreasing probability of reaching higher
/// layers, keeping the graph sparse at the top.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn random_level(ml: f64) -> usize {
    let mut rng = rand::rng();
    let r: f64 = rng.random::<f64>();
    // Clamp to avoid -inf when r approaches 0.
    let u = r.max(f64::MIN_POSITIVE);
    (-u.ln() * ml).floor() as usize
}
