use std::sync::Arc;

pub mod distance;
pub mod serde;

pub use distance::{Distance, DistanceMetric, distance_for_metric};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::distance::{
        CosineDistance, DotProductDistance, EuclideanDistance, ManhattanDistance,
    };

    // --- Vector::dimension ---

    #[test]
    fn dimension_f32() {
        assert_eq!(Vector::F32(vec![1.0, 2.0, 3.0]).dimension(), 3);
    }

    #[test]
    fn dimension_i16() {
        assert_eq!(Vector::I16(vec![1, 2]).dimension(), 2);
    }

    // --- Vector::to_f32_vec ---

    #[test]
    fn to_f32_vec_from_i16() {
        assert_eq!(Vector::I16(vec![1, 2]).to_f32_vec(), vec![1.0f32, 2.0f32]);
    }

    #[test]
    fn to_f32_vec_from_f64() {
        let v = Vector::F64(vec![1.5, 2.5]);
        assert_eq!(v.to_f32_vec(), vec![1.5f32, 2.5f32]);
    }

    #[test]
    fn to_f32_vec_from_i32() {
        let v = Vector::I32(vec![10, 20]);
        assert_eq!(v.to_f32_vec(), vec![10.0f32, 20.0f32]);
    }

    // --- Vector::mem_size ---

    #[test]
    fn mem_size_f32() {
        assert_eq!(Vector::F32(vec![1.0, 2.0]).mem_size(), 8); // 2 * 4 bytes
    }

    #[test]
    fn mem_size_f64() {
        assert_eq!(Vector::F64(vec![1.0, 2.0]).mem_size(), 16); // 2 * 8 bytes
    }

    #[test]
    fn mem_size_i32() {
        assert_eq!(Vector::I32(vec![1, 2]).mem_size(), 8); // 2 * 4 bytes
    }

    #[test]
    fn mem_size_i16() {
        assert_eq!(Vector::I16(vec![1, 2]).mem_size(), 4); // 2 * 2 bytes
    }

    // --- SharedVector ---

    #[test]
    fn shared_vector_new_and_dimension() {
        let v = Vector::F32(vec![1.0, 2.0, 3.0]);
        let sv = SharedVector::new(v);
        assert_eq!(sv.dimension(), 3);
    }

    #[test]
    fn shared_vector_clone_does_not_deep_copy() {
        let v = Vector::F32(vec![1.0, 2.0]);
        let sv = SharedVector::new(v);
        let sv2 = sv.clone();
        // Both SharedVectors point to the same Arc — verify by equality
        assert_eq!(sv, sv2);
        // Arc::ptr_eq confirms same allocation
        assert!(Arc::ptr_eq(&sv.inner, &sv2.inner));
    }

    #[test]
    fn shared_vector_partial_eq() {
        let a = SharedVector::new(Vector::F32(vec![1.0, 2.0]));
        let b = SharedVector::new(Vector::F32(vec![1.0, 2.0]));
        let c = SharedVector::new(Vector::F32(vec![3.0, 4.0]));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // --- Distance: Cosine ---

    #[test]
    fn cosine_orthogonal_vectors_returns_1() {
        let d = CosineDistance;
        let result = d.compute(&[1.0, 0.0], &[0.0, 1.0]);
        assert!((result - 1.0).abs() < 1e-9, "expected 1.0, got {}", result);
    }

    #[test]
    fn cosine_identical_vectors_returns_0() {
        let d = CosineDistance;
        let result = d.compute(&[1.0, 0.0], &[1.0, 0.0]);
        assert!(result.abs() < 1e-9, "expected 0.0, got {}", result);
    }

    #[test]
    fn cosine_zero_norm_returns_1() {
        let d = CosineDistance;
        let result = d.compute(&[0.0, 0.0], &[1.0, 0.0]);
        assert!((result - 1.0).abs() < 1e-9, "expected 1.0, got {}", result);
    }

    // --- Distance: Euclidean ---

    #[test]
    fn euclidean_3_4_5_triangle() {
        let d = EuclideanDistance;
        let result = d.compute(&[0.0, 0.0], &[3.0, 4.0]);
        assert!((result - 5.0).abs() < 1e-9, "expected 5.0, got {}", result);
    }

    // --- Distance: DotProduct ---

    #[test]
    fn dot_product_negated() {
        let d = DotProductDistance;
        // dot([1,2], [3,4]) = 11; negated = -11
        let result = d.compute(&[1.0, 2.0], &[3.0, 4.0]);
        assert!((result - (-11.0)).abs() < 1e-9, "expected -11.0, got {}", result);
    }

    // --- Distance: Manhattan ---

    #[test]
    fn manhattan_distance() {
        let d = ManhattanDistance;
        let result = d.compute(&[1.0, 2.0], &[4.0, 6.0]);
        assert!((result - 7.0).abs() < 1e-9, "expected 7.0, got {}", result);
    }

    // --- Panic on mismatched lengths ---

    #[test]
    #[should_panic]
    fn cosine_panics_on_length_mismatch() {
        CosineDistance.compute(&[1.0], &[1.0, 2.0]);
    }

    #[test]
    #[should_panic]
    fn euclidean_panics_on_length_mismatch() {
        EuclideanDistance.compute(&[1.0, 2.0], &[1.0]);
    }

    #[test]
    #[should_panic]
    fn dot_product_panics_on_length_mismatch() {
        DotProductDistance.compute(&[1.0], &[1.0, 2.0]);
    }

    #[test]
    #[should_panic]
    fn manhattan_panics_on_length_mismatch() {
        ManhattanDistance.compute(&[1.0, 2.0], &[1.0]);
    }

    // --- distance_for_metric factory ---

    #[test]
    fn distance_for_metric_cosine() {
        let d = distance_for_metric(DistanceMetric::Cosine);
        assert_eq!(d.metric_type(), DistanceMetric::Cosine);
    }

    #[test]
    fn distance_for_metric_euclidean() {
        let d = distance_for_metric(DistanceMetric::Euclidean);
        assert_eq!(d.metric_type(), DistanceMetric::Euclidean);
    }

    #[test]
    fn distance_for_metric_dot_product() {
        let d = distance_for_metric(DistanceMetric::DotProduct);
        assert_eq!(d.metric_type(), DistanceMetric::DotProduct);
    }

    #[test]
    fn distance_for_metric_manhattan() {
        let d = distance_for_metric(DistanceMetric::Manhattan);
        assert_eq!(d.metric_type(), DistanceMetric::Manhattan);
    }

    // --- MsgPack serialization round-trips ---

    #[test]
    fn msgpack_roundtrip_f32() {
        let v = Vector::F32(vec![1.0, 2.0]);
        let bytes = rmp_serde::to_vec_named(&v).expect("serialize");
        let decoded: Vector = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(v, decoded);
    }

    #[test]
    fn msgpack_roundtrip_f64() {
        let v = Vector::F64(vec![1.0, 2.0, 3.0]);
        let bytes = rmp_serde::to_vec_named(&v).expect("serialize");
        let decoded: Vector = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(v, decoded);
    }

    #[test]
    fn msgpack_roundtrip_i32() {
        let v = Vector::I32(vec![10, -20, 30]);
        let bytes = rmp_serde::to_vec_named(&v).expect("serialize");
        let decoded: Vector = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(v, decoded);
    }

    #[test]
    fn msgpack_roundtrip_i16() {
        let v = Vector::I16(vec![100, 200, -50]);
        let bytes = rmp_serde::to_vec_named(&v).expect("serialize");
        let decoded: Vector = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(v, decoded);
    }

    #[test]
    fn msgpack_roundtrip_f32_byte_exact() {
        // Verify byte-exact round-trip per AC-10
        let v = Vector::F32(vec![1.0, 2.0]);
        let bytes1 = rmp_serde::to_vec_named(&v).expect("serialize 1");
        let decoded: Vector = rmp_serde::from_slice(&bytes1).expect("deserialize");
        let bytes2 = rmp_serde::to_vec_named(&decoded).expect("serialize 2");
        assert_eq!(bytes1, bytes2);
    }

    // --- Re-exports from crate root ---

    #[test]
    fn crate_root_reexports() {
        use crate::{Distance, DistanceMetric, SharedVector, Vector, distance_for_metric};
        let _v = Vector::F32(vec![1.0]);
        let _sv = SharedVector::new(Vector::F32(vec![1.0]));
        let _d: Box<dyn Distance> = distance_for_metric(DistanceMetric::Cosine);
    }
}

/// Vector value type supporting multiple numeric element types.
///
/// Immutable after construction — no mutation methods are provided.
/// Use `to_f32_vec()` before computing distances since the Distance trait
/// operates on `&[f32]` slices.
#[derive(Debug, Clone, PartialEq)]
pub enum Vector {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I32(Vec<i32>),
    I16(Vec<i16>),
}

impl Vector {
    /// Returns the number of dimensions (length of the inner vec).
    #[must_use]
    pub fn dimension(&self) -> usize {
        match self {
            Vector::F32(v) => v.len(),
            Vector::F64(v) => v.len(),
            Vector::I32(v) => v.len(),
            Vector::I16(v) => v.len(),
        }
    }

    /// Converts any variant to a new `Vec<f32>` for distance computation.
    ///
    /// I32 conversion is lossy for values exceeding f32 precision — acceptable
    /// for approximate distance computation. F64 truncation is also intentional.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn to_f32_vec(&self) -> Vec<f32> {
        match self {
            Vector::F32(v) => v.clone(),
            Vector::F64(v) => v.iter().map(|&x| x as f32).collect(),
            Vector::I32(v) => v.iter().map(|&x| x as f32).collect(),
            Vector::I16(v) => v.iter().map(|&x| f32::from(x)).collect(),
        }
    }

    /// Returns the raw data size in bytes (element count * element size).
    ///
    /// Does not include `Vec` struct overhead — used for cache eviction weighting.
    #[must_use]
    pub fn mem_size(&self) -> usize {
        match self {
            Vector::F32(v) => v.len() * std::mem::size_of::<f32>(),
            Vector::F64(v) => v.len() * std::mem::size_of::<f64>(),
            Vector::I32(v) => v.len() * std::mem::size_of::<i32>(),
            Vector::I16(v) => v.len() * std::mem::size_of::<i16>(),
        }
    }
}

/// Arc-wrapped `Vector` with cached dimension for O(1) access.
///
/// Clone is cheap — does not deep-copy vector data (Arc semantics).
#[derive(Debug, Clone)]
pub struct SharedVector {
    inner: Arc<Vector>,
    dimension: usize,
}

impl SharedVector {
    /// Wraps a `Vector` in an Arc and caches the dimension.
    #[must_use]
    pub fn new(v: Vector) -> Self {
        let dimension = v.dimension();
        SharedVector {
            inner: Arc::new(v),
            dimension,
        }
    }

    /// Returns a reference to the inner `Vector`.
    #[must_use]
    pub fn vector(&self) -> &Vector {
        &self.inner
    }

    /// Returns the cached dimension (O(1)).
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Returns the approximate memory size of the vector data plus Arc overhead.
    ///
    /// Arc overhead is approximated as `3 * size_of::<usize>()` (2 atomic counters + 1 pointer),
    /// sufficient for cache eviction weighting.
    #[must_use]
    pub fn mem_size(&self) -> usize {
        self.inner.mem_size() + std::mem::size_of::<usize>() * 3
    }
}

impl PartialEq for SharedVector {
    fn eq(&self, other: &Self) -> bool {
        *self.inner == *other.inner
    }
}
