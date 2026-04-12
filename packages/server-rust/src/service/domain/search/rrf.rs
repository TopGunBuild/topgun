//! Reciprocal Rank Fusion (RRF) algorithm for merging ranked result lists.
//!
//! RRF is a pure, stateless function: it takes ranked lists from multiple
//! search methods and produces a single deduplicated ranking using the formula:
//!
//! ```text
//! RRF_score(d) = sum over all lists of 1 / (k + rank_i(d))
//! ```
//!
//! Keeping this module free of async, Arc, and service dependencies makes it
//! easy to unit-test and reason about in isolation.

use std::collections::HashMap;

/// A single entry from one search method's ranked results.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct RankedEntry {
    /// Record key.
    pub key: String,
    /// 1-based rank within the originating method's result list.
    pub rank: u32,
    /// Original score from the method (for transparency, not used in fusion).
    pub original_score: f64,
}

/// A result after RRF fusion.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct FusedEntry {
    pub key: String,
    /// Fused RRF score (sum of reciprocal ranks across methods).
    pub score: f64,
    /// Per-method original scores for transparency: (`list_index`, `original_score`).
    pub method_scores: Vec<(usize, f64)>,
}

/// Fuse multiple ranked lists using Reciprocal Rank Fusion.
///
/// # Errors
///
/// This function is infallible and never returns an error.
///
/// Returns results sorted descending by fused score, limited to `top_n`.
#[must_use]
pub fn fuse(ranked_lists: &[Vec<RankedEntry>], k: u32, top_n: usize) -> Vec<FusedEntry> {
    if ranked_lists.is_empty() || top_n == 0 {
        return Vec::new();
    }

    let mut accumulator: HashMap<String, FusedEntry> = HashMap::new();

    for (list_idx, list) in ranked_lists.iter().enumerate() {
        for entry in list {
            let rrf_contribution = 1.0 / (f64::from(k) + f64::from(entry.rank));
            let fused = accumulator.entry(entry.key.clone()).or_insert_with(|| FusedEntry {
                key: entry.key.clone(),
                score: 0.0,
                method_scores: Vec::new(),
            });
            fused.score += rrf_contribution;
            fused.method_scores.push((list_idx, entry.original_score));
        }
    }

    let mut results: Vec<FusedEntry> = accumulator.into_values().collect();
    // Sort descending by fused score, then by key for deterministic tie-breaking.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    results.truncate(top_n);
    results
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, rank: u32, score: f64) -> RankedEntry {
        RankedEntry {
            key: key.to_string(),
            rank,
            original_score: score,
        }
    }

    /// Single list passthrough: RRF score = 1/(k+rank) for each entry.
    #[test]
    fn test_single_list_passthrough() {
        let list = vec![entry("a", 1, 1.0), entry("b", 2, 0.9), entry("c", 3, 0.8)];
        let results = fuse(&[list], 60, 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].key, "a");
        let expected_a = 1.0 / 61.0;
        assert!((results[0].score - expected_a).abs() < 1e-10);
    }

    /// Two lists with overlapping keys: overlapping key gets contributions from both.
    #[test]
    fn test_two_lists_with_overlap() {
        let list_a = vec![entry("x", 1, 1.0), entry("y", 2, 0.8)];
        let list_b = vec![entry("y", 1, 0.9), entry("z", 2, 0.7)];
        let results = fuse(&[list_a, list_b], 60, 10);

        // "y" appears in both lists: 1/62 + 1/61
        let y = results.iter().find(|e| e.key == "y").expect("y must be present");
        let expected_y = 1.0 / 62.0 + 1.0 / 61.0;
        assert!((y.score - expected_y).abs() < 1e-10);
        assert_eq!(y.method_scores.len(), 2);

        // "x" appears in list 0 only: 1/61
        let x = results.iter().find(|e| e.key == "x").expect("x must be present");
        let expected_x = 1.0 / 61.0;
        assert!((x.score - expected_x).abs() < 1e-10);

        // Verify "y" is ranked first (highest fused score)
        assert_eq!(results[0].key, "y");
    }

    /// k parameter effect: larger k reduces differences between ranks.
    #[test]
    fn test_k_parameter_effect() {
        let list = vec![entry("a", 1, 1.0), entry("b", 100, 0.5)];

        // With k=1: 1/2 vs 1/101 — large difference
        let results_small_k = fuse(std::slice::from_ref(&list), 1, 10);
        let diff_small = results_small_k[0].score - results_small_k[1].score;

        // With k=1000: 1/1001 vs 1/1100 — small difference
        let results_large_k = fuse(&[list], 1000, 10);
        let diff_large = results_large_k[0].score - results_large_k[1].score;

        assert!(diff_small > diff_large, "larger k should reduce score differences");
    }

    /// `top_n` truncation: only the `top_n` results are returned.
    #[test]
    fn test_top_n_truncation() {
        let list = vec![
            entry("a", 1, 1.0),
            entry("b", 2, 0.9),
            entry("c", 3, 0.8),
            entry("d", 4, 0.7),
            entry("e", 5, 0.6),
        ];
        let results = fuse(&[list], 60, 3);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].key, "a");
        assert_eq!(results[1].key, "b");
        assert_eq!(results[2].key, "c");
    }

    /// Empty input returns empty result.
    #[test]
    fn test_empty_input_returns_empty() {
        let results = fuse(&[], 60, 10);
        assert!(results.is_empty());

        // Also test with an empty list inside
        let results2 = fuse(&[vec![]], 60, 10);
        assert!(results2.is_empty());
    }

    /// Acceptance criterion: rank 1 in list A + rank 3 in list B with k=60
    /// should give score = 1/61 + 1/63.
    #[test]
    fn test_acceptance_criterion_score() {
        let list_a = vec![entry("doc", 1, 1.0)];
        let list_b = vec![entry("doc", 3, 0.7)];
        let results = fuse(&[list_a, list_b], 60, 10);
        assert_eq!(results.len(), 1);
        let expected = 1.0 / 61.0 + 1.0 / 63.0;
        assert!((results[0].score - expected).abs() < 1e-15);
    }
}
