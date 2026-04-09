use ahash::AHashSet;

use crate::service::domain::index::hnsw::types::{DynamicSet, ElementId, HnswFlavor};

// ---------------------------------------------------------------------------
// ArraySet<N> — stack-allocated fixed-capacity neighbor set
// ---------------------------------------------------------------------------

/// Fixed-capacity neighbor set backed by a stack-allocated array.
///
/// Chosen over Vec for M8/M12/M16 flavors because neighbor sets are small
/// (<=32 entries) and allocation-free insertion matters on the hot insert path.
pub struct ArraySet<const N: usize> {
    slots: [Option<ElementId>; N],
    count: usize,
}

impl<const N: usize> ArraySet<N> {
    #[must_use]
    pub fn new() -> Self {
        ArraySet {
            slots: [None; N],
            count: 0,
        }
    }
}

impl<const N: usize> Default for ArraySet<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> DynamicSet for ArraySet<N> {
    fn insert(&mut self, id: ElementId) -> bool {
        // Reject if already present — duplicate edges waste capacity.
        for slot in self.slots.iter() {
            if *slot == Some(id) {
                return true;
            }
        }
        // Find first empty slot.
        for slot in self.slots.iter_mut() {
            if slot.is_none() {
                *slot = Some(id);
                self.count += 1;
                return true;
            }
        }
        // No empty slot — at capacity.
        false
    }

    fn remove(&mut self, id: &ElementId) -> bool {
        for slot in self.slots.iter_mut() {
            if *slot == Some(*id) {
                *slot = None;
                self.count -= 1;
                return true;
            }
        }
        false
    }

    fn contains(&mut self, id: &ElementId) -> bool {
        self.slots.iter().any(|s| *s == Some(*id))
    }

    fn iter(&self) -> Box<dyn Iterator<Item = ElementId> + '_> {
        Box::new(self.slots.iter().filter_map(|s| *s))
    }

    fn len(&self) -> usize {
        self.count
    }

    fn capacity(&self) -> usize {
        N
    }
}

// ---------------------------------------------------------------------------
// AHashSetWrapper — heap-allocated neighbor set for Custom flavor
// ---------------------------------------------------------------------------

/// Heap-backed neighbor set used by the Custom flavor when m exceeds 16.
///
/// AHashSet provides faster hashing than std HashMap for integer keys,
/// important when neighbor sets are accessed frequently during graph traversal.
pub struct AHashSetWrapper {
    inner: AHashSet<ElementId>,
    capacity: usize,
}

impl AHashSetWrapper {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        AHashSetWrapper {
            inner: AHashSet::with_capacity(capacity),
            capacity,
        }
    }
}

impl DynamicSet for AHashSetWrapper {
    fn insert(&mut self, id: ElementId) -> bool {
        if self.inner.contains(&id) {
            return true;
        }
        if self.inner.len() >= self.capacity {
            return false;
        }
        self.inner.insert(id);
        true
    }

    fn remove(&mut self, id: &ElementId) -> bool {
        self.inner.remove(id)
    }

    fn contains(&mut self, id: &ElementId) -> bool {
        self.inner.contains(id)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = ElementId> + '_> {
        Box::new(self.inner.iter().copied())
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn capacity(&self) -> usize {
        self.capacity
    }
}

// ---------------------------------------------------------------------------
// HnswFlavor::create_set factory
// ---------------------------------------------------------------------------

impl HnswFlavor {
    /// Creates the appropriate `DynamicSet` for this flavor.
    ///
    /// `is_layer0 = true` selects the larger m0 capacity (base layer has
    /// higher degree to improve recall of the beam search at layer 0).
    #[must_use]
    pub fn create_set(&self, is_layer0: bool) -> Box<dyn DynamicSet> {
        match self {
            HnswFlavor::M8 => {
                if is_layer0 {
                    Box::new(ArraySet::<16>::new())
                } else {
                    Box::new(ArraySet::<8>::new())
                }
            }
            HnswFlavor::M12 => {
                if is_layer0 {
                    Box::new(ArraySet::<24>::new())
                } else {
                    Box::new(ArraySet::<12>::new())
                }
            }
            HnswFlavor::M16 => {
                if is_layer0 {
                    Box::new(ArraySet::<32>::new())
                } else {
                    Box::new(ArraySet::<16>::new())
                }
            }
            HnswFlavor::Custom { m, m0 } => {
                let cap = if is_layer0 { *m0 as usize } else { *m as usize };
                Box::new(AHashSetWrapper::new(cap))
            }
        }
    }
}
