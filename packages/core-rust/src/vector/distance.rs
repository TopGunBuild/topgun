use serde::{Deserialize, Serialize};

/// Supported distance metrics for vector similarity search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DistanceMetric {
    Cosine,
    Euclidean,
    DotProduct,
    Manhattan,
}

/// Trait for computing distance between two float slices.
///
/// All implementations panic if `a.len() != b.len()`.
/// Callers use `Vector::to_f32_vec()` to convert other numeric types before calling.
pub trait Distance: Send + Sync {
    fn compute(&self, a: &[f32], b: &[f32]) -> f64;
    fn metric_type(&self) -> DistanceMetric;
}

/// Cosine distance: `1.0 - cosine_similarity(a, b)`.
///
/// Returns 1.0 if either vector has zero norm (orthogonal by convention).
pub struct CosineDistance;

/// Euclidean distance: `sqrt(sum((a_i - b_i)^2))`.
pub struct EuclideanDistance;

/// Dot product distance: `-dot(a, b)`.
///
/// Negated so that smaller values mean more similar, consistent with
/// min-distance convention used by nearest-neighbor search.
pub struct DotProductDistance;

/// Manhattan (L1) distance: `sum(|a_i - b_i|)`.
pub struct ManhattanDistance;

impl Distance for CosineDistance {
    fn compute(&self, a: &[f32], b: &[f32]) -> f64 {
        assert_eq!(a.len(), b.len(), "CosineDistance: slice lengths must match");
        let mut dot = 0.0f64;
        let mut norm_a = 0.0f64;
        let mut norm_b = 0.0f64;
        for (&ai, &bi) in a.iter().zip(b.iter()) {
            let ai = ai as f64;
            let bi = bi as f64;
            dot += ai * bi;
            norm_a += ai * ai;
            norm_b += bi * bi;
        }
        let norm_a = norm_a.sqrt();
        let norm_b = norm_b.sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            1.0
        } else {
            1.0 - (dot / (norm_a * norm_b))
        }
    }

    fn metric_type(&self) -> DistanceMetric {
        DistanceMetric::Cosine
    }
}

impl Distance for EuclideanDistance {
    fn compute(&self, a: &[f32], b: &[f32]) -> f64 {
        assert_eq!(a.len(), b.len(), "EuclideanDistance: slice lengths must match");
        let sum_sq: f64 = a
            .iter()
            .zip(b.iter())
            .map(|(&ai, &bi)| {
                let diff = ai as f64 - bi as f64;
                diff * diff
            })
            .sum();
        sum_sq.sqrt()
    }

    fn metric_type(&self) -> DistanceMetric {
        DistanceMetric::Euclidean
    }
}

impl Distance for DotProductDistance {
    fn compute(&self, a: &[f32], b: &[f32]) -> f64 {
        assert_eq!(a.len(), b.len(), "DotProductDistance: slice lengths must match");
        let dot: f64 = a
            .iter()
            .zip(b.iter())
            .map(|(&ai, &bi)| ai as f64 * bi as f64)
            .sum();
        -dot
    }

    fn metric_type(&self) -> DistanceMetric {
        DistanceMetric::DotProduct
    }
}

impl Distance for ManhattanDistance {
    fn compute(&self, a: &[f32], b: &[f32]) -> f64 {
        assert_eq!(a.len(), b.len(), "ManhattanDistance: slice lengths must match");
        a.iter()
            .zip(b.iter())
            .map(|(&ai, &bi)| (ai as f64 - bi as f64).abs())
            .sum()
    }

    fn metric_type(&self) -> DistanceMetric {
        DistanceMetric::Manhattan
    }
}

/// Returns a boxed `Distance` implementation for the given metric.
pub fn distance_for_metric(metric: DistanceMetric) -> Box<dyn Distance> {
    match metric {
        DistanceMetric::Cosine => Box::new(CosineDistance),
        DistanceMetric::Euclidean => Box::new(EuclideanDistance),
        DistanceMetric::DotProduct => Box::new(DotProductDistance),
        DistanceMetric::Manhattan => Box::new(ManhattanDistance),
    }
}
