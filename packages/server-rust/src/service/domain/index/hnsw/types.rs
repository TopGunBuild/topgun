/// Unique identifier for elements in the HNSW index.
///
/// u64 gives ample ID space without pointer overhead — callers manage
/// their own mapping from domain keys to ElementId.
pub type ElementId = u64;

/// Abstraction over neighbor-set implementations.
///
/// Two concrete implementations exist: `ArraySet<N>` (stack-allocated, fixed
/// capacity) and `AHashSetWrapper` (heap-allocated, configurable capacity).
/// The trait lets `UndirectedGraph` and `Layer` work with either without
/// monomorphization bloat across the whole module.
pub trait DynamicSet: Send + Sync {
    /// Add a neighbor. Returns `false` when the set is already at capacity.
    fn insert(&mut self, id: ElementId) -> bool;

    /// Remove a neighbor. Returns `false` if the id was not present.
    fn remove(&mut self, id: &ElementId) -> bool;

    /// Returns `true` if the id is already in the set.
    fn contains(&mut self, id: &ElementId) -> bool;

    /// Iterate over all neighbors in unspecified order.
    fn iter(&self) -> Box<dyn Iterator<Item = ElementId> + '_>;

    /// Number of currently stored neighbors.
    fn len(&self) -> usize;

    /// Returns `true` when len() == 0.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Maximum number of neighbors this set can hold.
    fn capacity(&self) -> usize;
}

/// Controls which neighbor-selection heuristic is used during insertion.
///
/// Variants match the four combinations of the two extensions described in
/// the HNSW paper (Algorithm 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heuristic {
    /// Keep M closest neighbors — simplest strategy.
    Standard,
    /// Expand the candidate set with neighbors-of-candidates before selecting.
    Extended,
    /// After main selection, fill remaining slots from pruned candidates.
    KeepPruned,
    /// Both extended candidate expansion and keep-pruned filling.
    ExtendedAndKeep,
}

/// Construction and search parameters for an HNSW index.
///
/// `Default` is implemented manually so that `ml` is always derived from `m`
/// rather than left as 0.0, which would produce degenerate level assignments.
pub struct HnswParams {
    /// Vector dimensionality — all inserted vectors must match this.
    pub dimension: u16,
    /// Distance metric used by the index.
    pub distance: topgun_core::vector::DistanceMetric,
    /// Max number of neighbors per node on non-base layers.
    pub m: u8,
    /// Max number of neighbors per node on the base layer (layer 0).
    pub m0: u8,
    /// Size of the dynamic candidate list during construction.
    pub ef_construction: u16,
    /// Level-generation multiplier: `1.0 / ln(m)`.
    pub ml: f64,
    /// When true, neighbor selection expands candidates to neighbors-of-candidates.
    pub extend_candidates: bool,
    /// When true, pruned candidates fill remaining neighbor slots.
    pub keep_pruned_connections: bool,
}

impl Default for HnswParams {
    fn default() -> Self {
        // ml is derived from m so that level assignments follow the paper's
        // expected distribution.  Hard-coding 16 here keeps Default free of
        // circular references while matching the chosen default m value.
        HnswParams {
            dimension: 0,
            distance: topgun_core::vector::DistanceMetric::Cosine,
            m: 16,
            m0: 32,
            ef_construction: 200,
            ml: 1.0 / (16f64).ln(),
            extend_candidates: false,
            keep_pruned_connections: false,
        }
    }
}

/// Runtime flavor selector — determines which `DynamicSet` implementation is
/// used for neighbor storage.
///
/// Fixed variants (M8/M12/M16) use stack-allocated `ArraySet<N>` to avoid
/// heap allocation on the critical insert path.  `Custom` uses a heap-backed
/// `AHashSetWrapper` when m exceeds 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HnswFlavor {
    /// m=8, m0=16 — lowest memory footprint, suitable for high-D recall.
    M8,
    /// m=12, m0=24 — moderate memory/recall trade-off.
    M12,
    /// m=16, m0=32 — default; best general-purpose recall.
    M16,
    /// Arbitrary m and m0 — uses heap-allocated neighbor sets.
    Custom { m: u8, m0: u8 },
}

impl HnswFlavor {
    /// Returns the `m` (non-base-layer) neighbor limit for this flavor.
    #[must_use]
    pub fn m(&self) -> u8 {
        match self {
            HnswFlavor::M8 => 8,
            HnswFlavor::M12 => 12,
            HnswFlavor::M16 => 16,
            HnswFlavor::Custom { m, .. } => *m,
        }
    }

    /// Returns the `m0` (base-layer) neighbor limit for this flavor.
    #[must_use]
    pub fn m0(&self) -> u8 {
        match self {
            HnswFlavor::M8 => 16,
            HnswFlavor::M12 => 24,
            HnswFlavor::M16 => 32,
            HnswFlavor::Custom { m0, .. } => *m0,
        }
    }
}
