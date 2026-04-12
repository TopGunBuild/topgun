/// Unit tests for the HNSW module.
///
/// Tests cover all acceptance criteria: `ArraySet` capacity enforcement,
/// `HnswFlavor::create_set`, `DoublePriorityQueue` ordering, `UndirectedGraph`
/// bidirectionality, Heuristic variants, and Hnsw index public API.
#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::module_inception,
    clippy::uninlined_format_args,
    clippy::stable_sort_primitive,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]
mod tests {
    use topgun_core::vector::{DistanceMetric, SharedVector, Vector};

    use crate::service::domain::index::hnsw::flavor::{AHashSetWrapper, ArraySet};
    use crate::service::domain::index::hnsw::graph::UndirectedGraph;
    use crate::service::domain::index::hnsw::heuristic::{select_neighbors, DoublePriorityQueue};
    use crate::service::domain::index::hnsw::index::Hnsw;
    use crate::service::domain::index::hnsw::types::{
        DynamicSet, Heuristic, HnswFlavor, HnswParams,
    };

    // -----------------------------------------------------------------------
    // AC-7 + AC-8: ArraySet capacity enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn array_set_16_enforces_capacity() {
        let mut s = ArraySet::<16>::new();
        for i in 0u64..16 {
            assert!(s.insert(i), "insert {} should succeed", i);
        }
        // 17th insert should fail — capacity is 16.
        assert!(!s.insert(16), "insert beyond capacity should return false");
        assert_eq!(s.len(), 16);
    }

    #[test]
    fn array_set_32_enforces_capacity() {
        let mut s = ArraySet::<32>::new();
        for i in 0u64..32 {
            assert!(s.insert(i), "insert {} should succeed", i);
        }
        assert!(!s.insert(32), "insert beyond capacity should return false");
        assert_eq!(s.len(), 32);
    }

    #[test]
    fn array_set_duplicate_insert_returns_true_no_double_count() {
        let mut s = ArraySet::<8>::new();
        assert!(s.insert(1));
        assert!(s.insert(1)); // duplicate — still "inserted" (already present)
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn array_set_remove_works() {
        let mut s = ArraySet::<4>::new();
        s.insert(10);
        s.insert(20);
        assert!(s.remove(&10));
        assert_eq!(s.len(), 1);
        assert!(!s.remove(&99)); // not present
    }

    #[test]
    fn array_set_iter_returns_all() {
        let mut s = ArraySet::<4>::new();
        s.insert(1);
        s.insert(2);
        s.insert(3);
        let mut ids: Vec<u64> = s.iter().collect();
        ids.sort();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    // -----------------------------------------------------------------------
    // AC-9: HnswFlavor::create_set capacity
    // -----------------------------------------------------------------------

    #[test]
    fn flavor_m16_create_set_non_base_capacity_16() {
        let set = HnswFlavor::M16.create_set(false);
        assert_eq!(set.capacity(), 16);
    }

    #[test]
    fn flavor_m16_create_set_base_capacity_32() {
        let set = HnswFlavor::M16.create_set(true);
        assert_eq!(set.capacity(), 32);
    }

    #[test]
    fn flavor_m8_create_set_non_base_capacity_8() {
        let set = HnswFlavor::M8.create_set(false);
        assert_eq!(set.capacity(), 8);
    }

    #[test]
    fn flavor_m8_create_set_base_capacity_16() {
        let set = HnswFlavor::M8.create_set(true);
        assert_eq!(set.capacity(), 16);
    }

    #[test]
    fn flavor_m12_create_set_non_base_capacity_12() {
        let set = HnswFlavor::M12.create_set(false);
        assert_eq!(set.capacity(), 12);
    }

    #[test]
    fn flavor_custom_create_set_capacity() {
        let flavor = HnswFlavor::Custom { m: 20, m0: 40 };
        assert_eq!(flavor.create_set(false).capacity(), 20);
        assert_eq!(flavor.create_set(true).capacity(), 40);
    }

    // -----------------------------------------------------------------------
    // AHashSetWrapper capacity
    // -----------------------------------------------------------------------

    #[test]
    fn ahash_wrapper_enforces_capacity() {
        let mut s = AHashSetWrapper::new(4);
        for i in 0u64..4 {
            assert!(s.insert(i));
        }
        assert!(!s.insert(99));
        assert_eq!(s.len(), 4);
    }

    // -----------------------------------------------------------------------
    // AC-10: DoublePriorityQueue ordering
    // -----------------------------------------------------------------------

    #[test]
    fn dpq_pop_nearest_returns_minimum() {
        let mut q = DoublePriorityQueue::new();
        q.insert(10, 5.0);
        q.insert(20, 1.0);
        q.insert(30, 3.0);
        let (id, d) = q.pop_nearest().unwrap();
        assert_eq!(id, 20);
        assert!((d - 1.0).abs() < 1e-9);
    }

    #[test]
    fn dpq_pop_furthest_returns_maximum() {
        let mut q = DoublePriorityQueue::new();
        q.insert(10, 5.0);
        q.insert(20, 1.0);
        q.insert(30, 3.0);
        let (id, d) = q.pop_furthest().unwrap();
        assert_eq!(id, 10);
        assert!((d - 5.0).abs() < 1e-9);
    }

    #[test]
    fn dpq_peek_does_not_remove() {
        let mut q = DoublePriorityQueue::new();
        q.insert(1, 2.0);
        q.insert(2, 8.0);
        assert_eq!(q.len(), 2);
        let _ = q.peek_nearest();
        let _ = q.peek_furthest();
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn dpq_empty_returns_none() {
        let mut q = DoublePriorityQueue::new();
        assert!(q.pop_nearest().is_none());
        assert!(q.pop_furthest().is_none());
    }

    // -----------------------------------------------------------------------
    // AC-11: UndirectedGraph bidirectionality
    // -----------------------------------------------------------------------

    #[test]
    fn graph_add_edge_bidirectional() {
        let mut g = UndirectedGraph::new();
        g.add_node(1, HnswFlavor::M16.create_set(false));
        g.add_node(2, HnswFlavor::M16.create_set(false));
        g.add_edge(1, 2);
        // Both directions must be present.
        assert!(g.neighbors(&1).unwrap().iter().any(|id| id == 2));
        assert!(g.neighbors(&2).unwrap().iter().any(|id| id == 1));
    }

    #[test]
    fn graph_remove_node_cleans_back_edges() {
        let mut g = UndirectedGraph::new();
        for id in [1u64, 2, 3] {
            g.add_node(id, HnswFlavor::M16.create_set(false));
        }
        g.add_edge(1, 2);
        g.add_edge(1, 3);
        g.remove_node(1);
        assert!(!g.has_node(&1));
        // Node 2 should no longer have node 1 as a neighbor.
        let nbrs_of_2: Vec<u64> = g.neighbors(&2).unwrap().iter().collect();
        assert!(!nbrs_of_2.contains(&1));
    }

    // -----------------------------------------------------------------------
    // AC-1: empty index
    // -----------------------------------------------------------------------

    fn make_params(dim: u16) -> HnswParams {
        HnswParams {
            dimension: dim,
            distance: DistanceMetric::Euclidean,
            m: 16,
            m0: 32,
            ef_construction: 200,
            ml: 1.0 / (16f64).ln(),
            extend_candidates: false,
            keep_pruned_connections: false,
        }
    }

    fn make_vector(vals: Vec<f32>) -> SharedVector {
        SharedVector::new(Vector::F32(vals))
    }

    fn random_vector(dim: usize, seed_offset: u64) -> SharedVector {
        // Deterministic pseudo-random vectors using a simple LCG for portability.
        let mut state = 6_364_136_223_846_793_005_u64.wrapping_add(seed_offset);
        let vals: Vec<f32> = (0..dim)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        SharedVector::new(Vector::F32(vals))
    }

    #[test]
    fn hnsw_new_is_empty() {
        let h = Hnsw::new(make_params(4));
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC-2: insert 1000 random 128-dim vectors without panic
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_insert_1000_vectors_no_panic() {
        let mut h = Hnsw::new(make_params(128));
        for i in 0u64..1000 {
            h.insert(i, random_vector(128, i));
        }
        assert_eq!(h.len(), 1000);
    }

    // -----------------------------------------------------------------------
    // AC-3: search returns exactly k results sorted ascending
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_search_returns_k_sorted_ascending() {
        let mut h = Hnsw::new(make_params(16));
        for i in 0u64..200 {
            h.insert(i, random_vector(16, i));
        }
        let query: Vec<f32> = (0..16).map(|i| i as f32 * 0.1).collect();
        let results = h.search(&query, 10, 50);
        assert_eq!(results.len(), 10);
        // Verify ascending order.
        for w in results.windows(2) {
            assert!(
                w[0].1 <= w[1].1,
                "results not sorted: {} > {}",
                w[0].1,
                w[1].1
            );
        }
    }

    // -----------------------------------------------------------------------
    // AC-4: recall >= 90% at ef=100 vs brute-force on 1000 random 128-dim
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_recall_90_percent_at_ef100() {
        const N: u64 = 1000;
        const DIM: usize = 128;
        const K: usize = 10;
        const EF: usize = 100;

        let mut h = Hnsw::new(make_params(DIM as u16));
        let mut all_vecs: Vec<(u64, Vec<f32>)> = Vec::new();

        for i in 0..N {
            let sv = random_vector(DIM, i);
            let raw = sv.vector().to_f32_vec();
            h.insert(i, sv);
            all_vecs.push((i, raw));
        }

        // Pick query as a new random vector (not in the index).
        let query_sv = random_vector(DIM, N + 99999);
        let query_raw = query_sv.vector().to_f32_vec();

        // Brute-force ground truth (Euclidean).
        let mut brute: Vec<(u64, f64)> = all_vecs
            .iter()
            .map(|(id, v)| {
                let d: f64 = v
                    .iter()
                    .zip(query_raw.iter())
                    .map(|(a, b)| {
                        let diff = a - b;
                        (diff * diff) as f64
                    })
                    .sum::<f64>()
                    .sqrt();
                (*id, d)
            })
            .collect();
        brute.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let ground_truth: std::collections::HashSet<u64> =
            brute.iter().take(K).map(|(id, _)| *id).collect();

        let results = h.search(&query_raw, K, EF);
        let found: std::collections::HashSet<u64> = results.iter().map(|(id, _)| *id).collect();
        let overlap = found.intersection(&ground_truth).count();
        let recall = overlap as f64 / K as f64;

        assert!(
            recall >= 0.90,
            "recall {:.2} < 0.90 (overlap {}/{})",
            recall,
            overlap,
            K
        );
    }

    // -----------------------------------------------------------------------
    // AC-5: remove excludes element from search
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_remove_excludes_from_search() {
        let mut h = Hnsw::new(make_params(8));
        for i in 0u64..100 {
            h.insert(i, random_vector(8, i));
        }
        // Remove first 20 elements.
        for i in 0u64..20 {
            assert!(h.remove(i));
        }
        assert_eq!(h.len(), 80);

        let query: Vec<f32> = vec![0.0; 8];
        let results = h.search(&query, 50, 80);
        let ids: std::collections::HashSet<u64> = results.iter().map(|(id, _)| *id).collect();
        for i in 0u64..20 {
            assert!(
                !ids.contains(&i),
                "deleted element {} appeared in search results",
                i
            );
        }
    }

    // -----------------------------------------------------------------------
    // AC-6: optimize rebuilds graph, len == non-deleted count, search works
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_optimize_rebuilds_correctly() {
        let mut h = Hnsw::new(make_params(8));
        for i in 0u64..50 {
            h.insert(i, random_vector(8, i));
        }
        for i in 0u64..10 {
            h.remove(i);
        }
        assert_eq!(h.len(), 40);

        h.optimize();
        assert_eq!(h.len(), 40);

        // Search should still work post-optimize.
        let query: Vec<f32> = vec![0.0; 8];
        let results = h.search(&query, 5, 20);
        assert!(!results.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC-12: dimension mismatch panics
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "dimension mismatch")]
    fn hnsw_dimension_mismatch_panics() {
        let mut h = Hnsw::new(make_params(4));
        h.insert(0, make_vector(vec![1.0, 2.0, 3.0])); // 3-dim, not 4
    }

    // -----------------------------------------------------------------------
    // AC-13 + AC-14 + AC-15: Heuristic variants
    // -----------------------------------------------------------------------

    fn make_candidates() -> Vec<(u64, f64)> {
        vec![(1, 1.0), (2, 2.0), (3, 3.0), (4, 4.0), (5, 5.0)]
    }

    #[test]
    fn heuristic_standard_result_le_m() {
        let g = UndirectedGraph::new();
        let dist_fn = |_: u64, _: u64| 1.0f64;
        let result = select_neighbors(&make_candidates(), 3, Heuristic::Standard, &g, &dist_fn);
        assert!(result.len() <= 3);
    }

    #[test]
    fn heuristic_extended_result_le_m() {
        let mut g = UndirectedGraph::new();
        // Add a neighbor of candidate 1 that isn't in the original list.
        g.add_node(1, HnswFlavor::M8.create_set(false));
        g.add_node(99, HnswFlavor::M8.create_set(false));
        g.add_edge(1, 99);

        let dist_fn = |_: u64, _: u64| 0.5f64;
        let result = select_neighbors(&make_candidates(), 4, Heuristic::Extended, &g, &dist_fn);
        assert!(result.len() <= 4, "result len {} > 4", result.len());
    }

    #[test]
    fn heuristic_keep_pruned_fills_slots() {
        // With a tight m=2, KeepPruned should fill more slots than Standard.
        let g = UndirectedGraph::new();
        // dist_fn returns large distance so all candidates beyond first get pruned initially.
        let dist_fn = |a: u64, b: u64| if a == b { 0.0 } else { 100.0 };
        let result_std = select_neighbors(&make_candidates(), 3, Heuristic::Standard, &g, &dist_fn);
        let result_kp =
            select_neighbors(&make_candidates(), 3, Heuristic::KeepPruned, &g, &dist_fn);
        // KeepPruned should return at least as many as Standard.
        assert!(result_kp.len() >= result_std.len());
        assert!(result_kp.len() <= 3);
    }

    #[test]
    fn heuristic_extended_and_keep_result_le_m() {
        let g = UndirectedGraph::new();
        let dist_fn = |_: u64, _: u64| 1.0f64;
        let result = select_neighbors(
            &make_candidates(),
            3,
            Heuristic::ExtendedAndKeep,
            &g,
            &dist_fn,
        );
        assert!(result.len() <= 3);
    }

    // -----------------------------------------------------------------------
    // contains / double-remove behavior
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_contains_and_double_remove() {
        let mut h = Hnsw::new(make_params(4));
        h.insert(1, make_vector(vec![1.0, 0.0, 0.0, 0.0]));
        assert!(h.contains(&1));
        assert!(h.remove(1));
        assert!(!h.contains(&1));
        // Double remove should return false.
        assert!(!h.remove(1));
    }

    // -----------------------------------------------------------------------
    // search on empty index
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_search_empty_returns_empty() {
        let h = Hnsw::new(make_params(4));
        let results = h.search(&[1.0, 0.0, 0.0, 0.0], 5, 10);
        assert!(results.is_empty());
    }

    // -----------------------------------------------------------------------
    // HnswParams::default derived ml
    // -----------------------------------------------------------------------

    #[test]
    fn hnsw_params_default_ml() {
        let p = HnswParams::default();
        let expected = 1.0 / (16f64).ln();
        assert!(
            (p.ml - expected).abs() < 1e-10,
            "ml mismatch: {} vs {}",
            p.ml,
            expected
        );
    }
}
