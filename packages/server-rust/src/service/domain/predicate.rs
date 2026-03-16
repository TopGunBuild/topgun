//! `PredicateEngine` module providing pure-function predicate evaluation.
//!
//! Evaluates `PredicateNode` trees and legacy `where` clause filters against
//! `rmpv::Value` record data. Used by `QueryService` for initial query evaluation
//! and by `QueryMutationObserver` for standing query re-evaluation.

use std::collections::HashMap;
use std::hash::BuildHasher;

use topgun_core::messages::base::{PredicateNode, PredicateOp, Query, SortDirection};
use topgun_core::messages::query::QueryResultEntry;
use topgun_core::types::Value;

// ---------------------------------------------------------------------------
// Value conversion
// ---------------------------------------------------------------------------

/// Converts a `Value` (from `RecordValue::Lww`) to `rmpv::Value` for predicate evaluation.
///
/// Used by the query module for constructing `QueryUpdatePayload` values
/// during mutation observation and for initial query evaluation.
pub(crate) fn value_to_rmpv(value: &Value) -> rmpv::Value {
    match value {
        Value::Null => rmpv::Value::Nil,
        Value::Bool(b) => rmpv::Value::Boolean(*b),
        Value::Int(i) => rmpv::Value::Integer((*i).into()),
        Value::Float(f) => rmpv::Value::F64(*f),
        Value::String(s) => rmpv::Value::String(s.clone().into()),
        Value::Bytes(b) => rmpv::Value::Binary(b.clone()),
        Value::Array(a) => rmpv::Value::Array(a.iter().map(value_to_rmpv).collect()),
        Value::Map(m) => rmpv::Value::Map(
            m.iter()
                .map(|(k, v)| (rmpv::Value::String(k.clone().into()), value_to_rmpv(v)))
                .collect(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Predicate evaluation
// ---------------------------------------------------------------------------

/// Evaluates a `PredicateNode` tree against a record's value map.
///
/// The `data` parameter is the record's value as an `rmpv::Value` (expected
/// to be a Map for field-level access). Returns `false` if data is not a Map.
#[must_use]
pub fn evaluate_predicate(predicate: &PredicateNode, data: &rmpv::Value) -> bool {
    match predicate.op {
        // L2 combinators
        PredicateOp::And => {
            let children = predicate.children.as_deref().unwrap_or(&[]);
            children.iter().all(|child| evaluate_predicate(child, data))
        }
        PredicateOp::Or => {
            let children = predicate.children.as_deref().unwrap_or(&[]);
            children.iter().any(|child| evaluate_predicate(child, data))
        }
        PredicateOp::Not => {
            let children = predicate.children.as_deref().unwrap_or(&[]);
            if children.is_empty() {
                // Vacuously true if no children
                true
            } else {
                !evaluate_predicate(&children[0], data)
            }
        }
        // L3 deferred
        PredicateOp::Like | PredicateOp::Regex => false,
        // L1 leaf operators
        _ => evaluate_leaf(predicate, data),
    }
}

/// Evaluates a legacy `where` clause against a record's value map.
///
/// Each entry is treated as an exact equality check. All entries must match
/// (implicit AND). Returns `false` if `data` is not a Map.
#[must_use]
pub fn evaluate_where<S: BuildHasher>(
    where_clause: &HashMap<String, rmpv::Value, S>,
    data: &rmpv::Value,
) -> bool {
    let Some(map) = data.as_map() else {
        return false;
    };

    for (field, expected) in where_clause {
        match find_field_in_map(map, field) {
            Some(val) => {
                if !values_equal(val, expected) {
                    return false;
                }
            }
            None => return false,
        }
    }

    true
}

/// Evaluates a complete `Query` against a set of key-value entries.
///
/// Returns filtered, sorted, limited results.
/// Evaluation priority: predicate > where > match-all.
#[must_use]
pub fn execute_query(
    entries: Vec<(String, rmpv::Value)>,
    query: &Query,
) -> Vec<QueryResultEntry> {
    // 1. Filter
    let filtered: Vec<(String, rmpv::Value)> = entries
        .into_iter()
        .filter(|(_, data)| {
            if let Some(pred) = &query.predicate {
                evaluate_predicate(pred, data)
            } else if let Some(wh) = &query.r#where {
                evaluate_where(wh, data)
            } else {
                // No filter: match all
                true
            }
        })
        .collect();

    // 2. Sort
    let mut sorted = filtered;
    if let Some(sort_map) = &query.sort {
        // Use first entry only (HashMap iteration order is non-deterministic;
        // consistent with TS behavior using only primary sort field)
        if let Some((field, direction)) = sort_map.iter().next() {
            let field = field.clone();
            let desc = *direction == SortDirection::Desc;
            sorted.sort_by(|(_, a), (_, b)| {
                let va = a.as_map().and_then(|m| find_field_in_map(m, &field));
                let vb = b.as_map().and_then(|m| find_field_in_map(m, &field));
                let ord = compare_rmpv_values(va, vb);
                if desc { ord.reverse() } else { ord }
            });
        }
    }

    // 3. Limit
    let limited = if let Some(limit) = query.limit {
        sorted.into_iter().take(limit as usize).collect()
    } else {
        sorted
    };

    // 4. Convert to `QueryResultEntry`
    limited
        .into_iter()
        .map(|(key, value)| QueryResultEntry { key, value })
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Evaluates a leaf predicate (Eq, Neq, Gt, Gte, Lt, Lte).
fn evaluate_leaf(predicate: &PredicateNode, data: &rmpv::Value) -> bool {
    let Some(map) = data.as_map() else {
        return false;
    };

    let Some(attribute) = &predicate.attribute else {
        return false;
    };

    let Some(expected) = &predicate.value else {
        return false;
    };

    let Some(actual) = find_field_in_map(map, attribute) else {
        return false;
    };

    match predicate.op {
        PredicateOp::Eq => values_equal(actual, expected),
        PredicateOp::Neq => !values_equal(actual, expected),
        PredicateOp::Gt => compare_ordered(actual, expected) == Some(std::cmp::Ordering::Greater),
        PredicateOp::Gte => matches!(
            compare_ordered(actual, expected),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        PredicateOp::Lt => compare_ordered(actual, expected) == Some(std::cmp::Ordering::Less),
        PredicateOp::Lte => matches!(
            compare_ordered(actual, expected),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        _ => false,
    }
}

/// Finds a field by name in an `rmpv::Value::Map`.
fn find_field_in_map<'a>(
    map: &'a [(rmpv::Value, rmpv::Value)],
    field: &str,
) -> Option<&'a rmpv::Value> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(field))
        .map(|(_, v)| v)
}

/// Extracts an f64 from an `rmpv::Value` for numeric comparison.
///
/// Precision loss is acceptable here: this is for predicate evaluation,
/// not exact arithmetic. JavaScript also uses f64 for all numbers.
#[allow(clippy::cast_precision_loss)]
fn as_f64(v: &rmpv::Value) -> Option<f64> {
    match v {
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Some(n as f64)
            } else {
                i.as_u64().map(|n| n as f64)
            }
        }
        rmpv::Value::F64(f) => Some(*f),
        rmpv::Value::F32(f) => Some(f64::from(*f)),
        _ => None,
    }
}

/// Checks equality between two `rmpv::Value` instances with numeric coercion.
///
/// Exact float comparison is intentional here: matching TS/JS semantics where
/// `42 === 42.0` is true. Epsilon comparison would produce different results.
#[allow(clippy::float_cmp)]
fn values_equal(a: &rmpv::Value, b: &rmpv::Value) -> bool {
    // Try numeric comparison first (cross-type int/float)
    if let (Some(fa), Some(fb)) = (as_f64(a), as_f64(b)) {
        return fa == fb;
    }

    // String comparison
    if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        return sa == sb;
    }

    // Boolean comparison
    if let (rmpv::Value::Boolean(ba), rmpv::Value::Boolean(bb)) = (a, b) {
        return ba == bb;
    }

    // Nil comparison
    if a.is_nil() && b.is_nil() {
        return true;
    }

    // Fallback: structural equality
    a == b
}

/// Compares two `rmpv::Value` instances for ordering (Gt/Gte/Lt/Lte).
///
/// Returns `None` for incompatible types.
fn compare_ordered(a: &rmpv::Value, b: &rmpv::Value) -> Option<std::cmp::Ordering> {
    // Numeric comparison (cross-type: int-to-f64)
    if let (Some(fa), Some(fb)) = (as_f64(a), as_f64(b)) {
        return fa.partial_cmp(&fb);
    }

    // String comparison (lexicographic)
    if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        return Some(sa.cmp(sb));
    }

    // Incompatible types
    None
}

/// Compares two optional `rmpv::Value` references for sorting.
///
/// Missing field values sort last.
fn compare_rmpv_values(
    a: Option<&rmpv::Value>,
    b: Option<&rmpv::Value>,
) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater, // missing sorts last
        (Some(_), None) => std::cmp::Ordering::Less,    // missing sorts last
        (Some(va), Some(vb)) => {
            compare_ordered(va, vb).unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build an rmpv Map from key-value pairs.
    fn make_map(pairs: Vec<(&str, rmpv::Value)>) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (rmpv::Value::String(k.into()), v))
                .collect(),
        )
    }

    /// Helper: build a leaf predicate node.
    fn leaf(op: PredicateOp, attr: &str, value: rmpv::Value) -> PredicateNode {
        PredicateNode {
            op,
            attribute: Some(attr.to_string()),
            value: Some(value),
            children: None,
        }
    }

    /// Helper: build a combinator predicate node.
    fn combinator(op: PredicateOp, children: Vec<PredicateNode>) -> PredicateNode {
        PredicateNode {
            op,
            attribute: None,
            value: None,
            children: Some(children),
        }
    }

    // ---- value_to_rmpv tests ----

    #[test]
    fn value_to_rmpv_null() {
        assert_eq!(value_to_rmpv(&Value::Null), rmpv::Value::Nil);
    }

    #[test]
    fn value_to_rmpv_bool() {
        assert_eq!(
            value_to_rmpv(&Value::Bool(true)),
            rmpv::Value::Boolean(true)
        );
    }

    #[test]
    fn value_to_rmpv_int() {
        let result = value_to_rmpv(&Value::Int(42));
        assert_eq!(result, rmpv::Value::Integer(42.into()));
    }

    #[test]
    fn value_to_rmpv_float() {
        assert_eq!(value_to_rmpv(&Value::Float(2.72)), rmpv::Value::F64(2.72));
    }

    #[test]
    fn value_to_rmpv_string() {
        let result = value_to_rmpv(&Value::String("hello".to_string()));
        assert_eq!(result, rmpv::Value::String("hello".into()));
    }

    #[test]
    fn value_to_rmpv_bytes() {
        let result = value_to_rmpv(&Value::Bytes(vec![1, 2, 3]));
        assert_eq!(result, rmpv::Value::Binary(vec![1, 2, 3]));
    }

    #[test]
    fn value_to_rmpv_array() {
        let val = Value::Array(vec![Value::Int(1), Value::String("two".to_string())]);
        let result = value_to_rmpv(&val);
        assert_eq!(
            result,
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(1.into()),
                rmpv::Value::String("two".into()),
            ])
        );
    }

    #[test]
    fn value_to_rmpv_map() {
        use std::collections::BTreeMap;
        let mut m = BTreeMap::new();
        m.insert("age".to_string(), Value::Int(30));
        let result = value_to_rmpv(&Value::Map(m));
        assert_eq!(
            result,
            rmpv::Value::Map(vec![(
                rmpv::Value::String("age".into()),
                rmpv::Value::Integer(30.into()),
            )])
        );
    }

    // ---- evaluate_predicate L1 leaf tests (AC4) ----

    #[test]
    fn predicate_eq_integer() {
        let data = make_map(vec![("age", rmpv::Value::Integer(25.into()))]);
        let pred = leaf(PredicateOp::Eq, "age", rmpv::Value::Integer(25.into()));
        assert!(evaluate_predicate(&pred, &data));

        let pred_ne = leaf(PredicateOp::Eq, "age", rmpv::Value::Integer(30.into()));
        assert!(!evaluate_predicate(&pred_ne, &data));
    }

    #[test]
    fn predicate_neq_string() {
        let data = make_map(vec![("status", rmpv::Value::String("active".into()))]);
        let pred = leaf(
            PredicateOp::Neq,
            "status",
            rmpv::Value::String("inactive".into()),
        );
        assert!(evaluate_predicate(&pred, &data));

        let pred_eq = leaf(
            PredicateOp::Neq,
            "status",
            rmpv::Value::String("active".into()),
        );
        assert!(!evaluate_predicate(&pred_eq, &data));
    }

    #[test]
    fn predicate_gt_numeric() {
        let data = make_map(vec![("score", rmpv::Value::Integer(85.into()))]);
        let pred = leaf(PredicateOp::Gt, "score", rmpv::Value::Integer(80.into()));
        assert!(evaluate_predicate(&pred, &data));

        let pred_eq = leaf(PredicateOp::Gt, "score", rmpv::Value::Integer(85.into()));
        assert!(!evaluate_predicate(&pred_eq, &data));
    }

    #[test]
    fn predicate_lte_numeric() {
        let data = make_map(vec![("score", rmpv::Value::Integer(80.into()))]);
        let pred = leaf(PredicateOp::Lte, "score", rmpv::Value::Integer(80.into()));
        assert!(evaluate_predicate(&pred, &data));

        let pred_gt = leaf(PredicateOp::Lte, "score", rmpv::Value::Integer(79.into()));
        assert!(!evaluate_predicate(&pred_gt, &data));
    }

    #[test]
    fn predicate_missing_attribute_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(PredicateOp::Eq, "age", rmpv::Value::Integer(25.into()));
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_cross_type_numeric_comparison() {
        let data = make_map(vec![("score", rmpv::Value::Integer(100.into()))]);
        let pred = leaf(PredicateOp::Gte, "score", rmpv::Value::F64(99.5));
        assert!(evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_non_map_data_returns_false() {
        let data = rmpv::Value::String("not a map".into());
        let pred = leaf(PredicateOp::Eq, "key", rmpv::Value::Integer(1.into()));
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_string_ordering() {
        let data = make_map(vec![("name", rmpv::Value::String("banana".into()))]);
        let pred = leaf(
            PredicateOp::Gt,
            "name",
            rmpv::Value::String("apple".into()),
        );
        assert!(evaluate_predicate(&pred, &data));

        let pred_lt = leaf(
            PredicateOp::Lt,
            "name",
            rmpv::Value::String("cherry".into()),
        );
        assert!(evaluate_predicate(&pred_lt, &data));
    }

    #[test]
    fn predicate_incompatible_types_return_false() {
        let data = make_map(vec![("field", rmpv::Value::String("text".into()))]);
        let pred = leaf(PredicateOp::Gt, "field", rmpv::Value::Integer(5.into()));
        assert!(!evaluate_predicate(&pred, &data));
    }

    // ---- evaluate_predicate L2 combinator tests (AC5) ----

    #[test]
    fn predicate_and_all_match() {
        let data = make_map(vec![
            ("age", rmpv::Value::Integer(25.into())),
            ("active", rmpv::Value::Boolean(true)),
        ]);
        let pred = combinator(
            PredicateOp::And,
            vec![
                leaf(PredicateOp::Gte, "age", rmpv::Value::Integer(18.into())),
                leaf(PredicateOp::Eq, "active", rmpv::Value::Boolean(true)),
            ],
        );
        assert!(evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_and_one_fails() {
        let data = make_map(vec![
            ("age", rmpv::Value::Integer(15.into())),
            ("active", rmpv::Value::Boolean(true)),
        ]);
        let pred = combinator(
            PredicateOp::And,
            vec![
                leaf(PredicateOp::Gte, "age", rmpv::Value::Integer(18.into())),
                leaf(PredicateOp::Eq, "active", rmpv::Value::Boolean(true)),
            ],
        );
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_or_one_matches() {
        let data = make_map(vec![("role", rmpv::Value::String("admin".into()))]);
        let pred = combinator(
            PredicateOp::Or,
            vec![
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("admin".into())),
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("editor".into())),
            ],
        );
        assert!(evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_or_none_match() {
        let data = make_map(vec![("role", rmpv::Value::String("viewer".into()))]);
        let pred = combinator(
            PredicateOp::Or,
            vec![
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("admin".into())),
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("editor".into())),
            ],
        );
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_not_negates_child() {
        let data = make_map(vec![("banned", rmpv::Value::Boolean(true))]);
        let pred = combinator(
            PredicateOp::Not,
            vec![leaf(PredicateOp::Eq, "banned", rmpv::Value::Boolean(true))],
        );
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_not_vacuously_true_no_children() {
        let data = make_map(vec![]);
        let pred = combinator(PredicateOp::Not, vec![]);
        assert!(evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_and_empty_children_is_true() {
        let data = make_map(vec![]);
        let pred = combinator(PredicateOp::And, vec![]);
        assert!(evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_or_empty_children_is_false() {
        let data = make_map(vec![]);
        let pred = combinator(PredicateOp::Or, vec![]);
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_like_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(PredicateOp::Like, "name", rmpv::Value::String("Ali%".into()));
        assert!(!evaluate_predicate(&pred, &data));
    }

    #[test]
    fn predicate_regex_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(PredicateOp::Regex, "name", rmpv::Value::String("^Ali".into()));
        assert!(!evaluate_predicate(&pred, &data));
    }

    // ---- evaluate_where tests (AC6) ----

    #[test]
    fn where_all_match() {
        let data = make_map(vec![
            ("status", rmpv::Value::String("active".into())),
            ("role", rmpv::Value::String("admin".into())),
        ]);
        let mut wh = HashMap::new();
        wh.insert("status".to_string(), rmpv::Value::String("active".into()));
        wh.insert("role".to_string(), rmpv::Value::String("admin".into()));
        assert!(evaluate_where(&wh, &data));
    }

    #[test]
    fn where_one_mismatch() {
        let data = make_map(vec![
            ("status", rmpv::Value::String("active".into())),
            ("role", rmpv::Value::String("viewer".into())),
        ]);
        let mut wh = HashMap::new();
        wh.insert("status".to_string(), rmpv::Value::String("active".into()));
        wh.insert("role".to_string(), rmpv::Value::String("admin".into()));
        assert!(!evaluate_where(&wh, &data));
    }

    #[test]
    fn where_missing_field() {
        let data = make_map(vec![("status", rmpv::Value::String("active".into()))]);
        let mut wh = HashMap::new();
        wh.insert("status".to_string(), rmpv::Value::String("active".into()));
        wh.insert("role".to_string(), rmpv::Value::String("admin".into()));
        assert!(!evaluate_where(&wh, &data));
    }

    #[test]
    fn where_empty_clause_matches_everything() {
        let data = make_map(vec![("any", rmpv::Value::Integer(1.into()))]);
        let wh: HashMap<String, rmpv::Value> = HashMap::new();
        assert!(evaluate_where(&wh, &data));
    }

    #[test]
    fn where_non_map_data_returns_false() {
        let data = rmpv::Value::Integer(42.into());
        let mut wh = HashMap::new();
        wh.insert("key".to_string(), rmpv::Value::Integer(42.into()));
        assert!(!evaluate_where(&wh, &data));
    }

    // ---- execute_query tests (AC7) ----

    #[test]
    fn execute_query_filter_sort_limit() {
        let entries = vec![
            (
                "user-1".to_string(),
                make_map(vec![
                    ("name", rmpv::Value::String("Charlie".into())),
                    ("age", rmpv::Value::Integer(30.into())),
                ]),
            ),
            (
                "user-2".to_string(),
                make_map(vec![
                    ("name", rmpv::Value::String("Alice".into())),
                    ("age", rmpv::Value::Integer(25.into())),
                ]),
            ),
            (
                "user-3".to_string(),
                make_map(vec![
                    ("name", rmpv::Value::String("Bob".into())),
                    ("age", rmpv::Value::Integer(10.into())),
                ]),
            ),
            (
                "user-4".to_string(),
                make_map(vec![
                    ("name", rmpv::Value::String("Diana".into())),
                    ("age", rmpv::Value::Integer(35.into())),
                ]),
            ),
        ];

        let mut sort = HashMap::new();
        sort.insert("name".to_string(), SortDirection::Asc);

        let query = Query {
            predicate: Some(leaf(PredicateOp::Gte, "age", rmpv::Value::Integer(20.into()))),
            r#where: None,
            sort: Some(sort),
            limit: Some(2),
            cursor: None,
        };

        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].key, "user-2"); // Alice (age 25, sorted first by name)
        assert_eq!(results[1].key, "user-1"); // Charlie (age 30, sorted second)
    }

    #[test]
    fn execute_query_no_filter_returns_all() {
        let entries = vec![
            ("a".to_string(), make_map(vec![("x", rmpv::Value::Integer(1.into()))])),
            ("b".to_string(), make_map(vec![("x", rmpv::Value::Integer(2.into()))])),
        ];
        let query = Query::default();
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn execute_query_where_filter() {
        let entries = vec![
            ("a".to_string(), make_map(vec![("status", rmpv::Value::String("active".into()))])),
            ("b".to_string(), make_map(vec![("status", rmpv::Value::String("inactive".into()))])),
        ];
        let mut wh = HashMap::new();
        wh.insert("status".to_string(), rmpv::Value::String("active".into()));
        let query = Query {
            r#where: Some(wh),
            predicate: None,
            sort: None,
            limit: None,
            cursor: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "a");
    }

    #[test]
    fn execute_query_sort_desc() {
        let entries = vec![
            ("a".to_string(), make_map(vec![("score", rmpv::Value::Integer(10.into()))])),
            ("b".to_string(), make_map(vec![("score", rmpv::Value::Integer(30.into()))])),
            ("c".to_string(), make_map(vec![("score", rmpv::Value::Integer(20.into()))])),
        ];
        let mut sort = HashMap::new();
        sort.insert("score".to_string(), SortDirection::Desc);
        let query = Query {
            predicate: None,
            r#where: None,
            sort: Some(sort),
            limit: None,
            cursor: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results[0].key, "b"); // 30
        assert_eq!(results[1].key, "c"); // 20
        assert_eq!(results[2].key, "a"); // 10
    }

    #[test]
    fn execute_query_limit_only() {
        let entries = vec![
            ("a".to_string(), make_map(vec![])),
            ("b".to_string(), make_map(vec![])),
            ("c".to_string(), make_map(vec![])),
        ];
        let query = Query {
            predicate: None,
            r#where: None,
            sort: None,
            limit: Some(1),
            cursor: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn execute_query_predicate_takes_priority_over_where() {
        let entries = vec![
            (
                "a".to_string(),
                make_map(vec![
                    ("age", rmpv::Value::Integer(30.into())),
                    ("status", rmpv::Value::String("inactive".into())),
                ]),
            ),
            (
                "b".to_string(),
                make_map(vec![
                    ("age", rmpv::Value::Integer(10.into())),
                    ("status", rmpv::Value::String("active".into())),
                ]),
            ),
        ];
        let mut wh = HashMap::new();
        wh.insert("status".to_string(), rmpv::Value::String("active".into()));

        // Predicate filters by age >= 20; where would filter by status = active.
        // Predicate takes priority.
        let query = Query {
            predicate: Some(leaf(PredicateOp::Gte, "age", rmpv::Value::Integer(20.into()))),
            r#where: Some(wh),
            sort: None,
            limit: None,
            cursor: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "a");
    }

    #[test]
    fn execute_query_missing_sort_field_sorts_last() {
        let entries = vec![
            ("a".to_string(), make_map(vec![("score", rmpv::Value::Integer(10.into()))])),
            ("b".to_string(), make_map(vec![])), // no score field
            ("c".to_string(), make_map(vec![("score", rmpv::Value::Integer(5.into()))])),
        ];
        let mut sort = HashMap::new();
        sort.insert("score".to_string(), SortDirection::Asc);
        let query = Query {
            predicate: None,
            r#where: None,
            sort: Some(sort),
            limit: None,
            cursor: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results[0].key, "c"); // score 5
        assert_eq!(results[1].key, "a"); // score 10
        assert_eq!(results[2].key, "b"); // missing score sorts last
    }

    // ---- values_equal cross-type tests ----

    #[test]
    fn values_equal_int_and_float() {
        let a = rmpv::Value::Integer(42.into());
        let b = rmpv::Value::F64(42.0);
        assert!(values_equal(&a, &b));
    }

    #[test]
    fn values_equal_nil() {
        assert!(values_equal(&rmpv::Value::Nil, &rmpv::Value::Nil));
    }

    #[test]
    fn values_equal_bool() {
        assert!(values_equal(&rmpv::Value::Boolean(true), &rmpv::Value::Boolean(true)));
        assert!(!values_equal(&rmpv::Value::Boolean(true), &rmpv::Value::Boolean(false)));
    }
}
