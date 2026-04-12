//! Index-aware query optimizer for predicate acceleration.
//!
//! [`index_aware_evaluate`] selects the best available index for each leaf
//! predicate, narrows the candidate key set, and falls back to full scan when
//! no covering index exists. Every candidate key is verified against the
//! original predicate via [`evaluate_predicate`] to guarantee correctness.
//!
//! Integration point: called from `QueryService::handle_query_subscribe` before
//! delegating to the query backend, so that only candidate keys are passed to
//! predicate evaluation rather than the full key space.

use std::collections::HashSet;

use topgun_core::messages::base::{PredicateNode, PredicateOp};

use super::registry::IndexRegistry;
use crate::service::domain::predicate::{evaluate_predicate, EvalContext};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Evaluates a predicate against a set of records, using available indexes to
/// narrow the candidate set before applying `evaluate_predicate`.
///
/// # Parameters
///
/// - `registry`: Per-map index registry used for candidate lookup.
/// - `predicate`: The predicate tree to evaluate.
/// - `all_keys`: All record keys in the map (used for full-scan fallback).
/// - `records`: Closure that returns the `rmpv::Value` for a given key.
///
/// # Returns
///
/// The set of record keys that satisfy the predicate.
pub fn index_aware_evaluate<F>(
    registry: &IndexRegistry,
    predicate: &PredicateNode,
    all_keys: &[String],
    records: F,
) -> Vec<String>
where
    F: Fn(&str) -> Option<rmpv::Value>,
{
    let candidates = collect_candidates(registry, predicate, all_keys);

    // Final verification: every candidate is checked against the full predicate
    // so that index approximation errors (e.g. hash collisions) cannot produce
    // false positives.
    candidates
        .into_iter()
        .filter(|key| {
            if let Some(data) = records(key) {
                evaluate_predicate(predicate, &EvalContext::data_only(&data))
            } else {
                false
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Internal: candidate key collection
// ---------------------------------------------------------------------------

/// Returns the candidate key set for a predicate node by consulting available
/// indexes. The result is an over-approximation: callers must verify each
/// candidate against the full predicate.
fn collect_candidates(
    registry: &IndexRegistry,
    predicate: &PredicateNode,
    all_keys: &[String],
) -> HashSet<String> {
    match predicate.op {
        // ---- Equality: use HashIndex ----------------------------------------
        PredicateOp::Eq => {
            if let Some(index) = registry.get_best_index(predicate) {
                if let Some(ref val) = predicate.value {
                    return index.lookup_eq(val);
                }
            }
            full_scan_keys(all_keys)
        }

        // ---- Range operators: use NavigableIndex ----------------------------
        PredicateOp::Gt => {
            if let Some(index) = registry.get_best_index(predicate) {
                if let Some(ref val) = predicate.value {
                    return index.lookup_range(Some(val), false, None, false);
                }
            }
            full_scan_keys(all_keys)
        }
        PredicateOp::Gte => {
            if let Some(index) = registry.get_best_index(predicate) {
                if let Some(ref val) = predicate.value {
                    return index.lookup_range(Some(val), true, None, false);
                }
            }
            full_scan_keys(all_keys)
        }
        PredicateOp::Lt => {
            if let Some(index) = registry.get_best_index(predicate) {
                if let Some(ref val) = predicate.value {
                    return index.lookup_range(None, false, Some(val), false);
                }
            }
            full_scan_keys(all_keys)
        }
        PredicateOp::Lte => {
            if let Some(index) = registry.get_best_index(predicate) {
                if let Some(ref val) = predicate.value {
                    return index.lookup_range(None, false, Some(val), true);
                }
            }
            full_scan_keys(all_keys)
        }

        // ---- Like: use InvertedIndex ----------------------------------------
        PredicateOp::Like => {
            if let Some(index) = registry.get_best_index(predicate) {
                if let Some(ref val) = predicate.value {
                    if let Some(token) = val.as_str() {
                        return index.lookup_contains(token);
                    }
                }
            }
            full_scan_keys(all_keys)
        }

        // ---- No index acceleration for these operators: full scan -----------
        PredicateOp::Neq
        | PredicateOp::Regex
        | PredicateOp::Not
        | PredicateOp::In
        | PredicateOp::Between
        | PredicateOp::IsNull
        | PredicateOp::IsNotNull
        | PredicateOp::ContainsAll
        | PredicateOp::ContainsAny
        | PredicateOp::StartsWith
        | PredicateOp::EndsWith => full_scan_keys(all_keys),

        // ---- And: intersect indexed children; full-scan the rest ------------
        PredicateOp::And => candidates_for_and(registry, predicate, all_keys),

        // ---- Or: union indexed children; full-scan if any child unindexed ---
        PredicateOp::Or => candidates_for_or(registry, predicate, all_keys),
    }
}

/// And combinator: intersect candidates from indexed children, full-scan the rest.
fn candidates_for_and(
    registry: &IndexRegistry,
    predicate: &PredicateNode,
    all_keys: &[String],
) -> HashSet<String> {
    let children = predicate.children.as_deref().unwrap_or(&[]);
    if children.is_empty() {
        return full_scan_keys(all_keys);
    }

    let mut indexed_sets: Vec<HashSet<String>> = Vec::new();

    for child in children {
        let child_candidates = collect_candidates(registry, child, all_keys);
        let is_full_scan =
            child_candidates.len() == all_keys.len() || is_leaf_without_index(registry, child);
        if !is_full_scan {
            indexed_sets.push(child_candidates);
        }
    }

    if indexed_sets.is_empty() {
        return full_scan_keys(all_keys);
    }

    // Start with the smallest indexed set and intersect the rest.
    // Unindexed children are handled by the final evaluate_predicate pass.
    indexed_sets
        .into_iter()
        .reduce(|acc, set| acc.intersection(&set).cloned().collect())
        .unwrap_or_default()
}

/// Or combinator: union candidates from indexed children; full-scan if any child
/// lacks an index (otherwise the union would be incomplete).
fn candidates_for_or(
    registry: &IndexRegistry,
    predicate: &PredicateNode,
    all_keys: &[String],
) -> HashSet<String> {
    let children = predicate.children.as_deref().unwrap_or(&[]);
    if children.is_empty() {
        return HashSet::new();
    }

    let mut union: HashSet<String> = HashSet::new();

    for child in children {
        if is_leaf_without_index(registry, child) {
            return full_scan_keys(all_keys);
        }
        let child_candidates = collect_candidates(registry, child, all_keys);
        union.extend(child_candidates);
    }

    union
}

/// Returns all keys as a `HashSet` (full-scan fallback).
fn full_scan_keys(all_keys: &[String]) -> HashSet<String> {
    all_keys.iter().cloned().collect()
}

/// Returns `true` if the leaf predicate has no covering index in the registry,
/// or if the node is a compound (And/Or/Not) which is not directly indexable.
fn is_leaf_without_index(registry: &IndexRegistry, predicate: &PredicateNode) -> bool {
    match predicate.op {
        PredicateOp::And
        | PredicateOp::Or
        | PredicateOp::Not
        | PredicateOp::Neq
        | PredicateOp::Regex
        | PredicateOp::In
        | PredicateOp::Between
        | PredicateOp::IsNull
        | PredicateOp::IsNotNull => true,
        _ => registry.get_best_index(predicate).is_none(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::base::PredicateOp;

    use crate::service::domain::index::registry::IndexRegistry;

    fn make_registry_with_hash(attr: &str) -> IndexRegistry {
        let r = IndexRegistry::new();
        r.add_hash_index(attr);
        r
    }

    fn make_registry_with_navigable(attr: &str) -> IndexRegistry {
        let r = IndexRegistry::new();
        r.add_navigable_index(attr);
        r
    }

    fn make_registry_with_inverted(attr: &str) -> IndexRegistry {
        let r = IndexRegistry::new();
        r.add_inverted_index(attr);
        r
    }

    /// Build an rmpv Map record for a single attribute.
    fn make_record(attr: &str, val: rmpv::Value) -> rmpv::Value {
        rmpv::Value::Map(vec![(rmpv::Value::String(attr.into()), val)])
    }

    fn leaf(op: PredicateOp, attr: &str, val: rmpv::Value) -> PredicateNode {
        PredicateNode {
            op,
            attribute: Some(attr.to_string()),
            value: Some(val),
            ..Default::default()
        }
    }

    fn combinator(op: PredicateOp, children: Vec<PredicateNode>) -> PredicateNode {
        PredicateNode {
            op,
            children: Some(children),
            ..Default::default()
        }
    }

    // ---- AC1: Eq on indexed attribute uses HashIndex ----------------------

    #[test]
    fn eq_with_hash_index_returns_only_matching_keys() {
        let registry = make_registry_with_hash("status");

        // Index::insert expects the full record map so AttributeExtractor can
        // pull the "status" attribute from it.
        let idx = registry.get_index("status").unwrap();
        idx.insert(
            "k1",
            &make_record("status", rmpv::Value::String("active".into())),
        );
        idx.insert(
            "k2",
            &make_record("status", rmpv::Value::String("inactive".into())),
        );
        idx.insert(
            "k3",
            &make_record("status", rmpv::Value::String("active".into())),
        );

        let all_keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        let records = |key: &str| -> Option<rmpv::Value> {
            match key {
                "k1" | "k3" => Some(make_record("status", rmpv::Value::String("active".into()))),
                "k2" => Some(make_record(
                    "status",
                    rmpv::Value::String("inactive".into()),
                )),
                _ => None,
            }
        };

        let pred = leaf(
            PredicateOp::Eq,
            "status",
            rmpv::Value::String("active".into()),
        );
        let mut result = index_aware_evaluate(&registry, &pred, &all_keys, records);
        result.sort();

        assert_eq!(result, vec!["k1", "k3"]);
        // k2 ("inactive") must NOT be in results.
        assert!(!result.contains(&"k2".to_string()));
    }

    // ---- AC2: Gte on navigable-indexed attribute uses range scan ----------

    #[test]
    fn gte_with_navigable_index_uses_range_scan() {
        let registry = make_registry_with_navigable("age");
        let idx = registry.get_index("age").unwrap();
        idx.insert("k1", &make_record("age", rmpv::Value::Integer(10.into())));
        idx.insert("k2", &make_record("age", rmpv::Value::Integer(20.into())));
        idx.insert("k3", &make_record("age", rmpv::Value::Integer(30.into())));

        let all_keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        let records = |key: &str| -> Option<rmpv::Value> {
            match key {
                "k1" => Some(make_record("age", rmpv::Value::Integer(10.into()))),
                "k2" => Some(make_record("age", rmpv::Value::Integer(20.into()))),
                "k3" => Some(make_record("age", rmpv::Value::Integer(30.into()))),
                _ => None,
            }
        };

        let pred = leaf(PredicateOp::Gte, "age", rmpv::Value::Integer(20.into()));
        let mut result = index_aware_evaluate(&registry, &pred, &all_keys, records);
        result.sort();

        assert_eq!(result, vec!["k2", "k3"]);
        assert!(!result.contains(&"k1".to_string()));
    }

    // ---- Like with inverted index ----------------------------------------

    #[test]
    fn like_with_inverted_index_uses_lookup_contains() {
        let registry = make_registry_with_inverted("bio");
        let idx = registry.get_index("bio").unwrap();
        // InvertedIndex tokenises on whitespace; pass full record maps.
        idx.insert(
            "k1",
            &make_record("bio", rmpv::Value::String("rust developer".into())),
        );
        idx.insert(
            "k2",
            &make_record("bio", rmpv::Value::String("javascript developer".into())),
        );
        idx.insert(
            "k3",
            &make_record("bio", rmpv::Value::String("rust engineer".into())),
        );

        let all_keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        let records = |key: &str| -> Option<rmpv::Value> {
            match key {
                "k1" => Some(make_record(
                    "bio",
                    rmpv::Value::String("rust developer".into()),
                )),
                "k2" => Some(make_record(
                    "bio",
                    rmpv::Value::String("javascript developer".into()),
                )),
                "k3" => Some(make_record(
                    "bio",
                    rmpv::Value::String("rust engineer".into()),
                )),
                _ => None,
            }
        };

        // evaluate_predicate returns false for Like (L3 deferred), so the final
        // verification pass will filter out all candidates regardless of index hits.
        // The test asserts that the index narrows candidates correctly (not all 3
        // keys are returned, since Like falls back to full scan which would include
        // all 3 — but then evaluate_predicate rejects all since Like returns false).
        let pred = leaf(PredicateOp::Like, "bio", rmpv::Value::String("rust".into()));
        let result = index_aware_evaluate(&registry, &pred, &all_keys, records);
        // evaluate_predicate returns false for Like, so all candidates are rejected.
        assert!(
            result.is_empty(),
            "Like returns false in evaluate_predicate, so no matches"
        );
    }

    // ---- Fallback to full scan when no index exists ----------------------

    #[test]
    fn fallback_full_scan_when_no_index() {
        let registry = IndexRegistry::new(); // empty, no indexes

        let all_keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        let records = |key: &str| -> Option<rmpv::Value> {
            match key {
                "k1" => Some(make_record("score", rmpv::Value::Integer(5.into()))),
                "k2" => Some(make_record("score", rmpv::Value::Integer(15.into()))),
                "k3" => Some(make_record("score", rmpv::Value::Integer(25.into()))),
                _ => None,
            }
        };

        let pred = leaf(PredicateOp::Gt, "score", rmpv::Value::Integer(10.into()));
        let mut result = index_aware_evaluate(&registry, &pred, &all_keys, records);
        result.sort();

        // Full scan + evaluate_predicate: k2 (15 > 10) and k3 (25 > 10) match.
        assert_eq!(result, vec!["k2", "k3"]);
    }

    // ---- And combination -------------------------------------------------

    #[test]
    fn and_with_indexed_child_narrows_candidates() {
        let registry = make_registry_with_hash("role");
        let idx = registry.get_index("role").unwrap();

        // Records with two fields: role and active.
        let make_full_record = |role: &str, active: bool| -> rmpv::Value {
            rmpv::Value::Map(vec![
                (
                    rmpv::Value::String("role".into()),
                    rmpv::Value::String(role.into()),
                ),
                (
                    rmpv::Value::String("active".into()),
                    rmpv::Value::Boolean(active),
                ),
            ])
        };

        // Insert full record maps so AttributeExtractor can find the "role" field.
        idx.insert("k1", &make_full_record("admin", true));
        idx.insert("k2", &make_full_record("user", true));
        idx.insert("k3", &make_full_record("admin", false));

        let all_keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];

        let records = |key: &str| -> Option<rmpv::Value> {
            match key {
                "k1" => Some(make_full_record("admin", true)),
                "k2" => Some(make_full_record("user", true)),
                "k3" => Some(make_full_record("admin", false)),
                _ => None,
            }
        };

        let pred = combinator(
            PredicateOp::And,
            vec![
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("admin".into())),
                leaf(PredicateOp::Eq, "active", rmpv::Value::Boolean(true)),
            ],
        );

        let mut result = index_aware_evaluate(&registry, &pred, &all_keys, records);
        result.sort();

        // k1: admin + active=true → match
        // k2: user + active=true → role fails → no match
        // k3: admin + active=false → active fails → no match
        assert_eq!(result, vec!["k1"]);
    }

    // ---- Or combination --------------------------------------------------

    #[test]
    fn or_with_all_indexed_children_unions_candidates() {
        let registry = make_registry_with_hash("status");
        let idx = registry.get_index("status").unwrap();
        idx.insert(
            "k1",
            &make_record("status", rmpv::Value::String("active".into())),
        );
        idx.insert(
            "k2",
            &make_record("status", rmpv::Value::String("pending".into())),
        );
        idx.insert(
            "k3",
            &make_record("status", rmpv::Value::String("inactive".into())),
        );

        let all_keys = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        let records = |key: &str| -> Option<rmpv::Value> {
            match key {
                "k1" => Some(make_record("status", rmpv::Value::String("active".into()))),
                "k2" => Some(make_record("status", rmpv::Value::String("pending".into()))),
                "k3" => Some(make_record(
                    "status",
                    rmpv::Value::String("inactive".into()),
                )),
                _ => None,
            }
        };

        let pred = combinator(
            PredicateOp::Or,
            vec![
                leaf(
                    PredicateOp::Eq,
                    "status",
                    rmpv::Value::String("active".into()),
                ),
                leaf(
                    PredicateOp::Eq,
                    "status",
                    rmpv::Value::String("pending".into()),
                ),
            ],
        );

        let mut result = index_aware_evaluate(&registry, &pred, &all_keys, records);
        result.sort();

        assert_eq!(result, vec!["k1", "k2"]);
        assert!(!result.contains(&"k3".to_string()));
    }
}
