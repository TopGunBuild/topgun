//! `PredicateEngine` module providing pure-function predicate evaluation.
//!
//! Evaluates `PredicateNode` trees and legacy `where` clause filters against
//! `rmpv::Value` record data. Used by `QueryService` for initial query evaluation
//! and by `QueryMutationObserver` for standing query re-evaluation.

use std::collections::HashMap;
use std::hash::BuildHasher;

use regex::Regex;
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
// Evaluation context
// ---------------------------------------------------------------------------

/// Evaluation context for predicate resolution.
///
/// Carries the authenticated principal (as `rmpv::Value` for uniform dot-path
/// access) and the record being evaluated.
pub struct EvalContext<'a> {
    pub auth: Option<&'a rmpv::Value>,
    pub data: &'a rmpv::Value,
}

impl<'a> EvalContext<'a> {
    /// Creates a data-only context with no auth. Backward-compatible shorthand.
    #[must_use]
    pub fn data_only(data: &'a rmpv::Value) -> Self {
        EvalContext { auth: None, data }
    }
}

// ---------------------------------------------------------------------------
// Predicate evaluation
// ---------------------------------------------------------------------------

/// Evaluates a `PredicateNode` tree against an evaluation context.
///
/// The `ctx.data` value is expected to be an `rmpv::Value::Map` for field-level
/// access. Returns `false` if data is not a Map. `ctx.auth` is used when the
/// predicate contains a `value_ref` pointing to the `auth` namespace.
#[must_use]
pub fn evaluate_predicate(predicate: &PredicateNode, ctx: &EvalContext) -> bool {
    match predicate.op {
        // L2 combinators
        PredicateOp::And => {
            let children = predicate.children.as_deref().unwrap_or(&[]);
            children.iter().all(|child| evaluate_predicate(child, ctx))
        }
        PredicateOp::Or => {
            let children = predicate.children.as_deref().unwrap_or(&[]);
            children.iter().any(|child| evaluate_predicate(child, ctx))
        }
        PredicateOp::Not => {
            let children = predicate.children.as_deref().unwrap_or(&[]);
            if children.is_empty() {
                // Vacuously true if no children
                true
            } else {
                !evaluate_predicate(&children[0], ctx)
            }
        }
        // Null-check operators (don't require a value field)
        PredicateOp::IsNull | PredicateOp::IsNotNull => evaluate_null_check(predicate, ctx),
        // L1 leaf operators (require attribute + value)
        _ => evaluate_leaf(predicate, ctx),
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
pub fn execute_query(entries: Vec<(String, rmpv::Value)>, query: &Query) -> Vec<QueryResultEntry> {
    // 1. Filter
    let filtered: Vec<(String, rmpv::Value)> = entries
        .into_iter()
        .filter(|(_, data)| {
            if let Some(pred) = &query.predicate {
                evaluate_predicate(pred, &EvalContext::data_only(data))
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
                if desc {
                    ord.reverse()
                } else {
                    ord
                }
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

/// Evaluates a leaf predicate (Eq, Neq, Gt, Gte, Lt, Lte, Like, Regex, In, Between).
///
/// Attribute lookup uses dot-path traversal to support nested fields (e.g., `"address.city"`).
/// Comparison value is resolved from `value_ref` (if set) or `value` field. If `value_ref`
/// cannot be resolved (unknown namespace or missing field), the predicate returns `false`.
fn evaluate_leaf(predicate: &PredicateNode, ctx: &EvalContext) -> bool {
    let Some(attribute) = &predicate.attribute else {
        return false;
    };

    // Resolve attribute via dot-path traversal against ctx.data
    let segments: Vec<&str> = attribute.split('.').collect();
    let Some(actual) = resolve_dot_path(ctx.data, &segments) else {
        return false;
    };

    // Resolve comparison value: value_ref takes precedence over value
    let expected = if let Some(ref_str) = &predicate.value_ref {
        let Some(resolved) = resolve_value_ref(ref_str, ctx) else {
            return false;
        };
        resolved
    } else if let Some(val) = &predicate.value {
        val.clone()
    } else {
        return false;
    };

    match predicate.op {
        PredicateOp::Eq => values_equal(&actual, &expected),
        PredicateOp::Neq => !values_equal(&actual, &expected),
        PredicateOp::Gt => compare_ordered(&actual, &expected) == Some(std::cmp::Ordering::Greater),
        PredicateOp::Gte => matches!(
            compare_ordered(&actual, &expected),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
        PredicateOp::Lt => compare_ordered(&actual, &expected) == Some(std::cmp::Ordering::Less),
        PredicateOp::Lte => matches!(
            compare_ordered(&actual, &expected),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        PredicateOp::Like => evaluate_like(&actual, &expected),
        PredicateOp::Regex => evaluate_regex(&actual, &expected),
        PredicateOp::In => evaluate_in(&actual, &expected),
        PredicateOp::Between => evaluate_between(&actual, &expected),
        PredicateOp::ContainsAll => evaluate_contains_all(&actual, &expected),
        PredicateOp::ContainsAny => evaluate_contains_any(&actual, &expected),
        PredicateOp::StartsWith => evaluate_starts_with(&actual, &expected),
        PredicateOp::EndsWith => evaluate_ends_with(&actual, &expected),
        _ => false,
    }
}

/// Evaluates `IsNull` / `IsNotNull` operators (field presence check, no `value` required).
///
/// Attribute lookup uses dot-path traversal to support nested fields (e.g., `"address.city"`).
/// The `auth` namespace is resolved against `ctx.auth`: bare `"auth"` checks whether auth
/// context itself is absent; `"auth.X"` checks whether a field in the auth map is absent.
fn evaluate_null_check(predicate: &PredicateNode, ctx: &EvalContext) -> bool {
    let Some(attribute) = &predicate.attribute else {
        return false;
    };

    if attribute == "auth" {
        // Bare "auth" — check whether the auth context itself is absent.
        let is_null = ctx.auth.is_none();
        return match predicate.op {
            PredicateOp::IsNull => is_null,
            PredicateOp::IsNotNull => !is_null,
            _ => false,
        };
    } else if let Some(rest) = attribute.strip_prefix("auth.") {
        // auth.X — resolve within the auth map.
        return match ctx.auth {
            Some(auth_val) => {
                let segments: Vec<&str> = rest.split('.').collect();
                let field = resolve_dot_path(auth_val, &segments);
                let is_null = field.is_none() || field.as_ref().is_some_and(rmpv::Value::is_nil);
                match predicate.op {
                    PredicateOp::IsNull => is_null,
                    PredicateOp::IsNotNull => !is_null,
                    _ => false,
                }
            }
            // No auth context at all — any auth field is effectively null.
            None => matches!(predicate.op, PredicateOp::IsNull),
        };
    }

    // Data namespace (existing behavior).
    let segments: Vec<&str> = attribute.split('.').collect();
    let field = resolve_dot_path(ctx.data, &segments);
    let is_null = field.is_none() || field.as_ref().is_some_and(rmpv::Value::is_nil);

    match predicate.op {
        PredicateOp::IsNull => is_null,
        PredicateOp::IsNotNull => !is_null,
        _ => false,
    }
}

/// Evaluates LIKE pattern matching (SQL wildcards, case-insensitive).
///
/// `%` matches any sequence of characters; `_` matches exactly one character.
/// Matching is case-insensitive to align with TS client behaviour.
fn evaluate_like(actual: &rmpv::Value, pattern_val: &rmpv::Value) -> bool {
    let (Some(text), Some(pattern)) = (actual.as_str(), pattern_val.as_str()) else {
        return false;
    };

    // Escape all regex metacharacters in the raw pattern, then convert
    // SQL wildcards: '%' -> '.*', '_' -> '.'
    let escaped = regex::escape(pattern);
    let regex_str = escaped.replace('%', ".*").replace('_', ".");
    let full_pattern = format!("^(?i){regex_str}$");

    Regex::new(&full_pattern).is_ok_and(|re| re.is_match(text))
}

/// Evaluates REGEX pattern matching (case-sensitive by default).
///
/// Users may embed `(?i)` in the pattern for case-insensitive matching.
/// Compilation failures return `false` rather than panicking.
fn evaluate_regex(actual: &rmpv::Value, pattern_val: &rmpv::Value) -> bool {
    let (Some(text), Some(pattern)) = (actual.as_str(), pattern_val.as_str()) else {
        return false;
    };

    Regex::new(pattern).is_ok_and(|re| re.is_match(text))
}

/// Evaluates IN operator with bidirectional array membership.
///
/// - If `allowed_val` is an array: checks whether `actual` appears in `allowed_val` (scalar-in-array).
/// - If `actual` is an array and `allowed_val` is not: checks whether `allowed_val` appears in `actual` (array-contains-scalar).
/// - Otherwise: returns `false`.
///
/// The second case supports the `contains` parser keyword, which emits a `PredicateOp::In` node
/// with swapped attribute/value positions so `data.tags contains 'vip'` works without a new op variant.
fn evaluate_in(actual: &rmpv::Value, allowed_val: &rmpv::Value) -> bool {
    match (allowed_val, actual) {
        (rmpv::Value::Array(arr), _) => arr.iter().any(|item| values_equal(actual, item)),
        (_, rmpv::Value::Array(arr)) => arr.iter().any(|item| values_equal(item, allowed_val)),
        _ => false,
    }
}

/// Evaluates BETWEEN operator: field value must be within [low, high] inclusive.
fn evaluate_between(actual: &rmpv::Value, range_val: &rmpv::Value) -> bool {
    let rmpv::Value::Array(range) = range_val else {
        return false;
    };

    if range.len() != 2 {
        return false;
    }

    let low = &range[0];
    let high = &range[1];

    let gte_low = matches!(
        compare_ordered(actual, low),
        Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
    );
    let lte_high = matches!(
        compare_ordered(actual, high),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    );

    gte_low && lte_high
}

/// Evaluates `containsAll`: checks that `actual` array contains every element of `expected`.
///
/// `actual` must be an array; returns `false` if not. `expected` may be an array or scalar;
/// a scalar is treated as a single-element array (vacuous truth: empty expected returns `true`).
fn evaluate_contains_all(actual: &rmpv::Value, expected: &rmpv::Value) -> bool {
    let rmpv::Value::Array(actual_arr) = actual else {
        return false;
    };

    let expected_items: Vec<&rmpv::Value> = match expected {
        rmpv::Value::Array(arr) => arr.iter().collect(),
        scalar => vec![scalar],
    };

    // Vacuous truth: every element of an empty set is trivially contained.
    expected_items
        .iter()
        .all(|item| actual_arr.iter().any(|a| values_equal(a, item)))
}

/// Evaluates `containsAny`: checks that `actual` array contains at least one element of `expected`.
///
/// `actual` must be an array; returns `false` if not. `expected` may be an array or scalar;
/// a scalar is treated as a single-element array (empty expected returns `false`).
fn evaluate_contains_any(actual: &rmpv::Value, expected: &rmpv::Value) -> bool {
    let rmpv::Value::Array(actual_arr) = actual else {
        return false;
    };

    let expected_items: Vec<&rmpv::Value> = match expected {
        rmpv::Value::Array(arr) => arr.iter().collect(),
        scalar => vec![scalar],
    };

    expected_items
        .iter()
        .any(|item| actual_arr.iter().any(|a| values_equal(a, item)))
}

/// Evaluates `startsWith`: checks that `actual` string starts with `prefix` string.
///
/// Both arguments must be strings; returns `false` if either is not. Case-sensitive.
fn evaluate_starts_with(actual: &rmpv::Value, prefix: &rmpv::Value) -> bool {
    let (Some(actual_str), Some(prefix_str)) = (actual.as_str(), prefix.as_str()) else {
        return false;
    };
    actual_str.starts_with(prefix_str)
}

/// Evaluates `endsWith`: checks that `actual` string ends with `suffix` string.
///
/// Both arguments must be strings; returns `false` if either is not. Case-sensitive.
fn evaluate_ends_with(actual: &rmpv::Value, suffix: &rmpv::Value) -> bool {
    let (Some(actual_str), Some(suffix_str)) = (actual.as_str(), suffix.as_str()) else {
        return false;
    };
    actual_str.ends_with(suffix_str)
}

/// Traverses a nested `rmpv::Value::Map` using dot-path segments.
///
/// Iterates over `segments`, descending into Map entries at each step.
/// Returns `None` if any segment is missing or if an intermediate value is not a Map.
/// Returns a clone of the final value to avoid lifetime entanglement with the root.
fn resolve_dot_path(root: &rmpv::Value, segments: &[&str]) -> Option<rmpv::Value> {
    let mut current = root;
    for segment in segments {
        let map = current.as_map()?;
        current = map
            .iter()
            .find(|(k, _)| k.as_str() == Some(segment))
            .map(|(_, v)| v)?;
    }
    Some(current.clone())
}

/// Resolves a variable reference string against an `EvalContext`.
///
/// The `ref_str` format is `"namespace.path"` where:
/// - `"auth"` namespace maps to `ctx.auth` (returns `None` if auth is `None`)
/// - `"data"` namespace maps to `ctx.data`
/// - Any other namespace returns `None` (unknown namespace = safe false)
///
/// After the namespace is resolved, the remaining path segments are traversed
/// via `resolve_dot_path`. An empty path after the namespace returns the root value.
fn resolve_value_ref(ref_str: &str, ctx: &EvalContext) -> Option<rmpv::Value> {
    // Split on first '.' to separate namespace from rest
    let (namespace, rest_path) = match ref_str.find('.') {
        Some(idx) => (&ref_str[..idx], &ref_str[idx + 1..]),
        None => (ref_str, ""),
    };

    let root: &rmpv::Value = match namespace {
        "auth" => ctx.auth?,
        "data" => ctx.data,
        _ => return None,
    };

    if rest_path.is_empty() {
        return Some(root.clone());
    }

    let segments: Vec<&str> = rest_path.split('.').collect();
    resolve_dot_path(root, &segments)
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
fn compare_rmpv_values(a: Option<&rmpv::Value>, b: Option<&rmpv::Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater, // missing sorts last
        (Some(_), None) => std::cmp::Ordering::Less,    // missing sorts last
        (Some(va), Some(vb)) => compare_ordered(va, vb).unwrap_or(std::cmp::Ordering::Equal),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::doc_markdown)]
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
            ..Default::default()
        }
    }

    /// Helper: build a combinator predicate node.
    fn combinator(op: PredicateOp, children: Vec<PredicateNode>) -> PredicateNode {
        PredicateNode {
            op,
            children: Some(children),
            ..Default::default()
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
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let pred_ne = leaf(PredicateOp::Eq, "age", rmpv::Value::Integer(30.into()));
        assert!(!evaluate_predicate(
            &pred_ne,
            &EvalContext::data_only(&data)
        ));
    }

    #[test]
    fn predicate_neq_string() {
        let data = make_map(vec![("status", rmpv::Value::String("active".into()))]);
        let pred = leaf(
            PredicateOp::Neq,
            "status",
            rmpv::Value::String("inactive".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let pred_eq = leaf(
            PredicateOp::Neq,
            "status",
            rmpv::Value::String("active".into()),
        );
        assert!(!evaluate_predicate(
            &pred_eq,
            &EvalContext::data_only(&data)
        ));
    }

    #[test]
    fn predicate_gt_numeric() {
        let data = make_map(vec![("score", rmpv::Value::Integer(85.into()))]);
        let pred = leaf(PredicateOp::Gt, "score", rmpv::Value::Integer(80.into()));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let pred_eq = leaf(PredicateOp::Gt, "score", rmpv::Value::Integer(85.into()));
        assert!(!evaluate_predicate(
            &pred_eq,
            &EvalContext::data_only(&data)
        ));
    }

    #[test]
    fn predicate_lte_numeric() {
        let data = make_map(vec![("score", rmpv::Value::Integer(80.into()))]);
        let pred = leaf(PredicateOp::Lte, "score", rmpv::Value::Integer(80.into()));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let pred_gt = leaf(PredicateOp::Lte, "score", rmpv::Value::Integer(79.into()));
        assert!(!evaluate_predicate(
            &pred_gt,
            &EvalContext::data_only(&data)
        ));
    }

    #[test]
    fn predicate_missing_attribute_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(PredicateOp::Eq, "age", rmpv::Value::Integer(25.into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_cross_type_numeric_comparison() {
        let data = make_map(vec![("score", rmpv::Value::Integer(100.into()))]);
        let pred = leaf(PredicateOp::Gte, "score", rmpv::Value::F64(99.5));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_non_map_data_returns_false() {
        let data = rmpv::Value::String("not a map".into());
        let pred = leaf(PredicateOp::Eq, "key", rmpv::Value::Integer(1.into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_string_ordering() {
        let data = make_map(vec![("name", rmpv::Value::String("banana".into()))]);
        let pred = leaf(PredicateOp::Gt, "name", rmpv::Value::String("apple".into()));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let pred_lt = leaf(
            PredicateOp::Lt,
            "name",
            rmpv::Value::String("cherry".into()),
        );
        assert!(evaluate_predicate(&pred_lt, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_incompatible_types_return_false() {
        let data = make_map(vec![("field", rmpv::Value::String("text".into()))]);
        let pred = leaf(PredicateOp::Gt, "field", rmpv::Value::Integer(5.into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
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
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
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
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_or_one_matches() {
        let data = make_map(vec![("role", rmpv::Value::String("admin".into()))]);
        let pred = combinator(
            PredicateOp::Or,
            vec![
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("admin".into())),
                leaf(
                    PredicateOp::Eq,
                    "role",
                    rmpv::Value::String("editor".into()),
                ),
            ],
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_or_none_match() {
        let data = make_map(vec![("role", rmpv::Value::String("viewer".into()))]);
        let pred = combinator(
            PredicateOp::Or,
            vec![
                leaf(PredicateOp::Eq, "role", rmpv::Value::String("admin".into())),
                leaf(
                    PredicateOp::Eq,
                    "role",
                    rmpv::Value::String("editor".into()),
                ),
            ],
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_not_negates_child() {
        let data = make_map(vec![("banned", rmpv::Value::Boolean(true))]);
        let pred = combinator(
            PredicateOp::Not,
            vec![leaf(PredicateOp::Eq, "banned", rmpv::Value::Boolean(true))],
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_not_vacuously_true_no_children() {
        let data = make_map(vec![]);
        let pred = combinator(PredicateOp::Not, vec![]);
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_and_empty_children_is_true() {
        let data = make_map(vec![]);
        let pred = combinator(PredicateOp::And, vec![]);
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_or_empty_children_is_false() {
        let data = make_map(vec![]);
        let pred = combinator(PredicateOp::Or, vec![]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- Like tests (AC1) ----

    #[test]
    fn predicate_like_percent_at_end() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Like,
            "name",
            rmpv::Value::String("Ali%".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let data2 = make_map(vec![("name", rmpv::Value::String("Bob".into()))]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data2)));
    }

    #[test]
    fn predicate_like_percent_at_start() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Like,
            "name",
            rmpv::Value::String("%ice".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let data2 = make_map(vec![("name", rmpv::Value::String("Bob".into()))]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data2)));
    }

    #[test]
    fn predicate_like_percent_both_sides() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Like,
            "name",
            rmpv::Value::String("%li%".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let data2 = make_map(vec![("name", rmpv::Value::String("Bob".into()))]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data2)));
    }

    #[test]
    fn predicate_like_underscore_single_char() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Like,
            "name",
            rmpv::Value::String("A_ice".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let data2 = make_map(vec![("name", rmpv::Value::String("Aice".into()))]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data2)));
    }

    #[test]
    fn predicate_like_empty_pattern_matches_empty_string() {
        let data = make_map(vec![("name", rmpv::Value::String("".into()))]);
        let pred = leaf(PredicateOp::Like, "name", rmpv::Value::String("".into()));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let data2 = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data2)));
    }

    #[test]
    fn predicate_like_case_insensitive() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Like,
            "name",
            rmpv::Value::String("ali%".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_like_non_string_field_returns_false() {
        let data = make_map(vec![("age", rmpv::Value::Integer(42.into()))]);
        let pred = leaf(PredicateOp::Like, "age", rmpv::Value::String("%".into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- Regex tests (AC2) ----

    #[test]
    fn predicate_regex_simple_match() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Regex,
            "name",
            rmpv::Value::String("^Ali".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));

        let data2 = make_map(vec![("name", rmpv::Value::String("Bob".into()))]);
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data2)));
    }

    #[test]
    fn predicate_regex_invalid_pattern_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        // `[invalid` is an unclosed character class -- invalid regex
        let pred = leaf(
            PredicateOp::Regex,
            "name",
            rmpv::Value::String("[invalid".into()),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_regex_non_string_field_returns_false() {
        let data = make_map(vec![("age", rmpv::Value::Integer(42.into()))]);
        let pred = leaf(
            PredicateOp::Regex,
            "age",
            rmpv::Value::String("\\d+".into()),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_regex_case_sensitive_by_default() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Regex,
            "name",
            rmpv::Value::String("^ali".into()),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_regex_inline_case_insensitive_flag() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = leaf(
            PredicateOp::Regex,
            "name",
            rmpv::Value::String("(?i)^ali".into()),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- In tests (AC3) ----

    #[test]
    fn predicate_in_value_present() {
        let data = make_map(vec![("age", rmpv::Value::Integer(2.into()))]);
        let pred = leaf(
            PredicateOp::In,
            "age",
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(1.into()),
                rmpv::Value::Integer(2.into()),
                rmpv::Value::Integer(3.into()),
            ]),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_in_value_absent() {
        let data = make_map(vec![("age", rmpv::Value::Integer(5.into()))]);
        let pred = leaf(
            PredicateOp::In,
            "age",
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(1.into()),
                rmpv::Value::Integer(2.into()),
                rmpv::Value::Integer(3.into()),
            ]),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_in_empty_list_returns_false() {
        let data = make_map(vec![("age", rmpv::Value::Integer(1.into()))]);
        let pred = leaf(PredicateOp::In, "age", rmpv::Value::Array(vec![]));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_in_cross_type_numeric() {
        // Integer field matched against float in list
        let data = make_map(vec![("age", rmpv::Value::Integer(2.into()))]);
        let pred = leaf(
            PredicateOp::In,
            "age",
            rmpv::Value::Array(vec![rmpv::Value::F64(2.0)]),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_in_non_array_value_returns_false() {
        let data = make_map(vec![("age", rmpv::Value::Integer(2.into()))]);
        let pred = leaf(PredicateOp::In, "age", rmpv::Value::Integer(2.into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- Between tests (AC4) ----

    #[test]
    fn predicate_between_value_in_range() {
        let data = make_map(vec![("age", rmpv::Value::Integer(25.into()))]);
        let pred = leaf(
            PredicateOp::Between,
            "age",
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(18.into()),
                rmpv::Value::Integer(65.into()),
            ]),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_between_value_below_range() {
        let data = make_map(vec![("age", rmpv::Value::Integer(10.into()))]);
        let pred = leaf(
            PredicateOp::Between,
            "age",
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(18.into()),
                rmpv::Value::Integer(65.into()),
            ]),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_between_value_above_range() {
        let data = make_map(vec![("age", rmpv::Value::Integer(70.into()))]);
        let pred = leaf(
            PredicateOp::Between,
            "age",
            rmpv::Value::Array(vec![
                rmpv::Value::Integer(18.into()),
                rmpv::Value::Integer(65.into()),
            ]),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_between_boundary_inclusive() {
        let data_low = make_map(vec![("age", rmpv::Value::Integer(18.into()))]);
        let data_high = make_map(vec![("age", rmpv::Value::Integer(65.into()))]);
        let range = rmpv::Value::Array(vec![
            rmpv::Value::Integer(18.into()),
            rmpv::Value::Integer(65.into()),
        ]);
        let pred_low = leaf(PredicateOp::Between, "age", range.clone());
        let pred_high = leaf(PredicateOp::Between, "age", range);
        assert!(evaluate_predicate(
            &pred_low,
            &EvalContext::data_only(&data_low)
        ));
        assert!(evaluate_predicate(
            &pred_high,
            &EvalContext::data_only(&data_high)
        ));
    }

    #[test]
    fn predicate_between_string_range() {
        let data = make_map(vec![("name", rmpv::Value::String("mango".into()))]);
        let pred = leaf(
            PredicateOp::Between,
            "name",
            rmpv::Value::Array(vec![
                rmpv::Value::String("apple".into()),
                rmpv::Value::String("orange".into()),
            ]),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_between_non_2_element_array_returns_false() {
        let data = make_map(vec![("age", rmpv::Value::Integer(25.into()))]);
        let pred = leaf(
            PredicateOp::Between,
            "age",
            rmpv::Value::Array(vec![rmpv::Value::Integer(18.into())]),
        );
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- IsNull tests (AC5) ----

    #[test]
    fn predicate_is_null_nil_field() {
        let data = make_map(vec![("name", rmpv::Value::Nil)]);
        let pred = PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some("name".to_string()),
            ..Default::default()
        };
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_is_null_missing_field() {
        let data = make_map(vec![]);
        let pred = PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some("name".to_string()),
            ..Default::default()
        };
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_is_null_non_nil_field_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some("name".to_string()),
            ..Default::default()
        };
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- IsNotNull tests (AC6) ----

    #[test]
    fn predicate_is_not_null_non_nil_field() {
        let data = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        let pred = PredicateNode {
            op: PredicateOp::IsNotNull,
            attribute: Some("name".to_string()),
            ..Default::default()
        };
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_is_not_null_nil_field_returns_false() {
        let data = make_map(vec![("name", rmpv::Value::Nil)]);
        let pred = PredicateNode {
            op: PredicateOp::IsNotNull,
            attribute: Some("name".to_string()),
            ..Default::default()
        };
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn predicate_is_not_null_missing_field_returns_false() {
        let data = make_map(vec![]);
        let pred = PredicateNode {
            op: PredicateOp::IsNotNull,
            attribute: Some("name".to_string()),
            ..Default::default()
        };
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
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
            predicate: Some(leaf(
                PredicateOp::Gte,
                "age",
                rmpv::Value::Integer(20.into()),
            )),
            r#where: None,
            sort: Some(sort),
            limit: Some(2),
            cursor: None,
            group_by: None,
        };

        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].key, "user-2"); // Alice (age 25, sorted first by name)
        assert_eq!(results[1].key, "user-1"); // Charlie (age 30, sorted second)
    }

    #[test]
    fn execute_query_no_filter_returns_all() {
        let entries = vec![
            (
                "a".to_string(),
                make_map(vec![("x", rmpv::Value::Integer(1.into()))]),
            ),
            (
                "b".to_string(),
                make_map(vec![("x", rmpv::Value::Integer(2.into()))]),
            ),
        ];
        let query = Query::default();
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn execute_query_where_filter() {
        let entries = vec![
            (
                "a".to_string(),
                make_map(vec![("status", rmpv::Value::String("active".into()))]),
            ),
            (
                "b".to_string(),
                make_map(vec![("status", rmpv::Value::String("inactive".into()))]),
            ),
        ];
        let mut wh = HashMap::new();
        wh.insert("status".to_string(), rmpv::Value::String("active".into()));
        let query = Query {
            r#where: Some(wh),
            predicate: None,
            sort: None,
            limit: None,
            cursor: None,
            group_by: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "a");
    }

    #[test]
    fn execute_query_sort_desc() {
        let entries = vec![
            (
                "a".to_string(),
                make_map(vec![("score", rmpv::Value::Integer(10.into()))]),
            ),
            (
                "b".to_string(),
                make_map(vec![("score", rmpv::Value::Integer(30.into()))]),
            ),
            (
                "c".to_string(),
                make_map(vec![("score", rmpv::Value::Integer(20.into()))]),
            ),
        ];
        let mut sort = HashMap::new();
        sort.insert("score".to_string(), SortDirection::Desc);
        let query = Query {
            predicate: None,
            r#where: None,
            sort: Some(sort),
            limit: None,
            cursor: None,
            group_by: None,
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
            group_by: None,
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
            predicate: Some(leaf(
                PredicateOp::Gte,
                "age",
                rmpv::Value::Integer(20.into()),
            )),
            r#where: Some(wh),
            sort: None,
            limit: None,
            cursor: None,
            group_by: None,
        };
        let results = execute_query(entries, &query);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "a");
    }

    #[test]
    fn execute_query_missing_sort_field_sorts_last() {
        let entries = vec![
            (
                "a".to_string(),
                make_map(vec![("score", rmpv::Value::Integer(10.into()))]),
            ),
            ("b".to_string(), make_map(vec![])), // no score field
            (
                "c".to_string(),
                make_map(vec![("score", rmpv::Value::Integer(5.into()))]),
            ),
        ];
        let mut sort = HashMap::new();
        sort.insert("score".to_string(), SortDirection::Asc);
        let query = Query {
            predicate: None,
            r#where: None,
            sort: Some(sort),
            limit: None,
            cursor: None,
            group_by: None,
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
        assert!(values_equal(
            &rmpv::Value::Boolean(true),
            &rmpv::Value::Boolean(true)
        ));
        assert!(!values_equal(
            &rmpv::Value::Boolean(true),
            &rmpv::Value::Boolean(false)
        ));
    }

    // ---- Variable reference tests (AC4, AC5, AC6, AC7, AC8) ----

    /// Build an auth rmpv::Value map for testing.
    fn make_auth(pairs: Vec<(&str, rmpv::Value)>) -> rmpv::Value {
        make_map(pairs)
    }

    /// AC4: auth.id resolves to the id field of ctx.auth when auth is Some.
    #[test]
    fn value_ref_auth_id_matches_owner() {
        let auth = make_auth(vec![("id", rmpv::Value::String("user-42".into()))]);
        let data = make_map(vec![("ownerId", rmpv::Value::String("user-42".into()))]);
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("ownerId".to_string()),
            value_ref: Some("auth.id".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC4: auth.id does NOT match when IDs differ.
    #[test]
    fn value_ref_auth_id_no_match_different_values() {
        let auth = make_auth(vec![("id", rmpv::Value::String("user-99".into()))]);
        let data = make_map(vec![("ownerId", rmpv::Value::String("user-42".into()))]);
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("ownerId".to_string()),
            value_ref: Some("auth.id".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// AC5: auth.roles resolves to the roles array of ctx.auth.
    #[test]
    fn value_ref_auth_roles_resolves_to_array() {
        let roles_array = rmpv::Value::Array(vec![
            rmpv::Value::String("admin".into()),
            rmpv::Value::String("editor".into()),
        ]);
        let auth = make_auth(vec![("roles", roles_array.clone())]);
        let data = make_map(vec![("assignedRoles", roles_array.clone())]);
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("assignedRoles".to_string()),
            value_ref: Some("auth.roles".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC6: data.address.city resolves to a nested field in ctx.data.
    #[test]
    fn value_ref_data_nested_field_resolves() {
        let address = rmpv::Value::Map(vec![(
            rmpv::Value::String("city".into()),
            rmpv::Value::String("Berlin".into()),
        )]);
        let data = make_map(vec![
            ("address", address),
            ("city", rmpv::Value::String("Berlin".into())),
        ]);
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("city".to_string()),
            value_ref: Some("data.address.city".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC7: unknown namespace (e.g., "env.DEBUG") causes predicate to return false.
    #[test]
    fn value_ref_unknown_namespace_returns_false() {
        let data = make_map(vec![("status", rmpv::Value::String("active".into()))]);
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("status".to_string()),
            value_ref: Some("env.DEBUG".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// AC8: auth.id with ctx.auth = None causes predicate to return false.
    #[test]
    fn value_ref_auth_with_none_auth_returns_false() {
        let data = make_map(vec![("ownerId", rmpv::Value::String("user-42".into()))]);
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("ownerId".to_string()),
            value_ref: Some("auth.id".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    // ---- Nested attribute dot-path tests (AC9) ----

    /// AC9: nested attribute "address.city" resolves for leaf comparison.
    #[test]
    fn nested_attribute_dot_path_leaf_comparison() {
        let address = rmpv::Value::Map(vec![(
            rmpv::Value::String("city".into()),
            rmpv::Value::String("Berlin".into()),
        )]);
        let data = make_map(vec![("address", address)]);
        let pred = leaf(
            PredicateOp::Eq,
            "address.city",
            rmpv::Value::String("Berlin".into()),
        );
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC9: nested attribute "address.city" does not match for different value.
    #[test]
    fn nested_attribute_dot_path_leaf_no_match() {
        let address = rmpv::Value::Map(vec![(
            rmpv::Value::String("city".into()),
            rmpv::Value::String("Paris".into()),
        )]);
        let data = make_map(vec![("address", address)]);
        let pred = leaf(
            PredicateOp::Eq,
            "address.city",
            rmpv::Value::String("Berlin".into()),
        );
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// AC9: IsNull with nested attribute "address.city" returns true when field is nil.
    #[test]
    fn nested_attribute_is_null_nil_value() {
        let address =
            rmpv::Value::Map(vec![(rmpv::Value::String("city".into()), rmpv::Value::Nil)]);
        let data = make_map(vec![("address", address)]);
        let pred = PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some("address.city".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC9: IsNull with nested attribute "address.city" returns false when field has a value.
    #[test]
    fn nested_attribute_is_null_with_value_returns_false() {
        let address = rmpv::Value::Map(vec![(
            rmpv::Value::String("city".into()),
            rmpv::Value::String("Berlin".into()),
        )]);
        let data = make_map(vec![("address", address)]);
        let pred = PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some("address.city".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// AC9: IsNotNull with nested attribute "address.city" returns true when field has a value.
    #[test]
    fn nested_attribute_is_not_null_with_value() {
        let address = rmpv::Value::Map(vec![(
            rmpv::Value::String("city".into()),
            rmpv::Value::String("Berlin".into()),
        )]);
        let data = make_map(vec![("address", address)]);
        let pred = PredicateNode {
            op: PredicateOp::IsNotNull,
            attribute: Some("address.city".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC9: IsNull returns true when nested attribute path is absent.
    #[test]
    fn nested_attribute_is_null_missing_path_returns_true() {
        let address = rmpv::Value::Map(vec![(
            rmpv::Value::String("zip".into()),
            rmpv::Value::String("10115".into()),
        )]);
        let data = make_map(vec![("address", address)]);
        let pred = PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some("address.city".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// value_ref takes precedence over value when both are set.
    #[test]
    fn value_ref_takes_precedence_over_value() {
        let auth = make_auth(vec![("id", rmpv::Value::String("user-42".into()))]);
        let data = make_map(vec![("ownerId", rmpv::Value::String("user-42".into()))]);
        // value is wrong, but value_ref is correct — value_ref should win
        let pred = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("ownerId".to_string()),
            value: Some(rmpv::Value::String("wrong-value".into())),
            value_ref: Some("auth.id".to_string()),
            ..Default::default()
        };
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// EvalContext::data_only is backward-compatible (auth = None).
    #[test]
    fn eval_context_data_only_is_backward_compatible() {
        let data = make_map(vec![("score", rmpv::Value::Integer(100.into()))]);
        let pred = leaf(PredicateOp::Gte, "score", rmpv::Value::Integer(50.into()));
        // Passing via data_only — same behavior as legacy 2-arg call
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- contains semantics tests (bidirectional evaluate_in) ----

    #[test]
    fn contains_array_field_has_matching_string() {
        // data.tags is ["vip", "premium"]; checking if it contains "vip" -> true
        let tags = rmpv::Value::Array(vec![
            rmpv::Value::String("vip".into()),
            rmpv::Value::String("premium".into()),
        ]);
        let data = make_map(vec![("tags", tags)]);
        // contains uses PredicateOp::In with the scalar as value and the array field as attribute
        let pred = leaf(PredicateOp::In, "tags", rmpv::Value::String("vip".into()));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn contains_array_field_does_not_have_scalar() {
        // data.tags is ["basic"]; checking if it contains "vip" -> false
        let tags = rmpv::Value::Array(vec![rmpv::Value::String("basic".into())]);
        let data = make_map(vec![("tags", tags)]);
        let pred = leaf(PredicateOp::In, "tags", rmpv::Value::String("vip".into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn contains_array_field_has_matching_integer() {
        // data.scores is [1, 2, 3]; checking if it contains 2 -> true
        let scores = rmpv::Value::Array(vec![
            rmpv::Value::Integer(1.into()),
            rmpv::Value::Integer(2.into()),
            rmpv::Value::Integer(3.into()),
        ]);
        let data = make_map(vec![("scores", scores)]);
        let pred = leaf(PredicateOp::In, "scores", rmpv::Value::Integer(2.into()));
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn contains_empty_array_returns_false() {
        // data.tags is []; any scalar -> false
        let data = make_map(vec![("tags", rmpv::Value::Array(vec![]))]);
        let pred = leaf(PredicateOp::In, "tags", rmpv::Value::String("vip".into()));
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn contains_non_array_field_with_scalar_returns_false() {
        // data.role is "admin" (not an array); neither side is array -> false
        let data = make_map(vec![("role", rmpv::Value::String("admin".into()))]);
        let pred = leaf(PredicateOp::In, "role", rmpv::Value::String("admin".into()));
        // Neither actual (String) nor allowed_val (String) is an array, so false
        assert!(!evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    #[test]
    fn in_scalar_checked_against_array_unchanged() {
        // Existing scalar-in-array semantics still work: data.role in [admin, editor]
        let data = make_map(vec![("role", rmpv::Value::String("admin".into()))]);
        let pred = leaf(
            PredicateOp::In,
            "role",
            rmpv::Value::Array(vec![
                rmpv::Value::String("admin".into()),
                rmpv::Value::String("editor".into()),
            ]),
        );
        assert!(evaluate_predicate(&pred, &EvalContext::data_only(&data)));
    }

    // ---- auth-namespace null check tests (AC11, AC12 from SPEC-174) ----

    fn is_null_pred(attribute: &str) -> PredicateNode {
        PredicateNode {
            op: PredicateOp::IsNull,
            attribute: Some(attribute.to_string()),
            ..Default::default()
        }
    }

    fn is_not_null_pred(attribute: &str) -> PredicateNode {
        PredicateNode {
            op: PredicateOp::IsNotNull,
            attribute: Some(attribute.to_string()),
            ..Default::default()
        }
    }

    /// AC11: IsNull on bare "auth" returns true when ctx.auth is None.
    #[test]
    fn is_null_auth_returns_true_when_auth_none() {
        let data = make_map(vec![]);
        let pred = is_null_pred("auth");
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// AC12: IsNull on bare "auth" returns false when ctx.auth is Some.
    #[test]
    fn is_null_auth_returns_false_when_auth_some() {
        let data = make_map(vec![]);
        let auth = make_auth(vec![("id", rmpv::Value::String("user1".into()))]);
        let pred = is_null_pred("auth");
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// IsNotNull on bare "auth" returns false when ctx.auth is None.
    #[test]
    fn is_not_null_auth_returns_false_when_auth_none() {
        let data = make_map(vec![]);
        let pred = is_not_null_pred("auth");
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// IsNotNull on bare "auth" returns true when ctx.auth is Some.
    #[test]
    fn is_not_null_auth_returns_true_when_auth_some() {
        let data = make_map(vec![]);
        let auth = make_auth(vec![("id", rmpv::Value::String("user1".into()))]);
        let pred = is_not_null_pred("auth");
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// IsNull on "auth.id" returns false when auth has a non-nil "id" field.
    #[test]
    fn is_null_auth_field_returns_false_when_field_present() {
        let data = make_map(vec![]);
        let auth = make_auth(vec![("id", rmpv::Value::String("user1".into()))]);
        let pred = is_null_pred("auth.id");
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(!evaluate_predicate(&pred, &ctx));
    }

    /// IsNull on "auth.id" returns true when auth is missing the "id" field.
    #[test]
    fn is_null_auth_field_returns_true_when_field_absent() {
        let data = make_map(vec![]);
        let auth = make_auth(vec![]); // no id field
        let pred = is_null_pred("auth.id");
        let ctx = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// IsNull on "auth.X" returns true when ctx.auth is None (no auth context at all).
    #[test]
    fn is_null_auth_field_returns_true_when_auth_none() {
        let data = make_map(vec![]);
        let pred = is_null_pred("auth.id");
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&pred, &ctx));
    }

    /// Round-trip: parse "data.public == true && auth == null" and evaluate with no auth.
    #[test]
    fn round_trip_public_and_auth_null() {
        use crate::service::policy::expr_parser::parse_permission_expr;
        let node = parse_permission_expr("data.public == true && auth == null")
            .expect("expression should parse");
        let data = make_map(vec![("public", rmpv::Value::Boolean(true))]);
        // No auth context — should be true (public == true AND auth == null).
        let ctx = EvalContext {
            auth: None,
            data: &data,
        };
        assert!(evaluate_predicate(&node, &ctx));
        // With auth context — should be false (auth is not null).
        let auth = make_auth(vec![("id", rmpv::Value::String("u1".into()))]);
        let ctx_with_auth = EvalContext {
            auth: Some(&auth),
            data: &data,
        };
        assert!(!evaluate_predicate(&node, &ctx_with_auth));
    }

    // ---- evaluate_contains_all tests ----

    fn arr(items: &[&str]) -> rmpv::Value {
        rmpv::Value::Array(
            items
                .iter()
                .map(|s| rmpv::Value::String((*s).into()))
                .collect(),
        )
    }

    /// AC5: containsAll true when data array has all expected elements.
    #[test]
    fn contains_all_true_subset() {
        let actual = arr(&["a", "b", "c"]);
        let expected = arr(&["a", "c"]);
        assert!(evaluate_contains_all(&actual, &expected));
    }

    /// AC5: containsAll false when expected has a missing element.
    #[test]
    fn contains_all_false_missing_element() {
        let actual = arr(&["a", "b", "c"]);
        let expected = arr(&["a", "d"]);
        assert!(!evaluate_contains_all(&actual, &expected));
    }

    /// Empty expected array returns true (vacuous truth).
    #[test]
    fn contains_all_empty_expected_returns_true() {
        let actual = arr(&["a", "b"]);
        let expected = rmpv::Value::Array(vec![]);
        assert!(evaluate_contains_all(&actual, &expected));
    }

    /// Non-array actual returns false (safe deny).
    #[test]
    fn contains_all_non_array_actual_returns_false() {
        let actual = rmpv::Value::String("not-an-array".into());
        let expected = arr(&["a"]);
        // AC9
        assert!(!evaluate_contains_all(&actual, &expected));
    }

    /// AC14: scalar RHS is treated as single-element array.
    #[test]
    fn contains_all_scalar_rhs_treated_as_array() {
        let actual = arr(&["admin", "editor"]);
        let expected = rmpv::Value::String("admin".into());
        assert!(evaluate_contains_all(&actual, &expected));

        let expected_miss = rmpv::Value::String("superuser".into());
        assert!(!evaluate_contains_all(&actual, &expected_miss));
    }

    // ---- evaluate_contains_any tests ----

    /// AC6: containsAny true when at least one element overlaps.
    #[test]
    fn contains_any_true_overlap() {
        let actual = arr(&["a", "b"]);
        let expected = arr(&["b", "x"]);
        assert!(evaluate_contains_any(&actual, &expected));
    }

    /// AC6: containsAny false when no overlap.
    #[test]
    fn contains_any_false_no_overlap() {
        let actual = arr(&["a", "b"]);
        let expected = arr(&["x", "y"]);
        assert!(!evaluate_contains_any(&actual, &expected));
    }

    /// Empty expected array returns false (no overlap possible).
    #[test]
    fn contains_any_empty_expected_returns_false() {
        let actual = arr(&["a", "b"]);
        let expected = rmpv::Value::Array(vec![]);
        assert!(!evaluate_contains_any(&actual, &expected));
    }

    /// Non-array actual returns false (safe deny).
    #[test]
    fn contains_any_non_array_actual_returns_false() {
        let actual = rmpv::Value::Integer(42.into());
        let expected = arr(&["a"]);
        assert!(!evaluate_contains_any(&actual, &expected));
    }

    /// Scalar RHS treated as single-element array.
    #[test]
    fn contains_any_scalar_rhs() {
        let actual = arr(&["editor", "viewer"]);
        let expected = rmpv::Value::String("editor".into());
        assert!(evaluate_contains_any(&actual, &expected));
    }

    // ---- evaluate_starts_with tests ----

    /// AC7: startsWith true for matching prefix.
    #[test]
    fn starts_with_true() {
        let actual = rmpv::Value::String("/public/docs".into());
        let prefix = rmpv::Value::String("/public".into());
        assert!(evaluate_starts_with(&actual, &prefix));
    }

    /// AC7: startsWith false for non-matching prefix.
    #[test]
    fn starts_with_false() {
        let actual = rmpv::Value::String("/private/docs".into());
        let prefix = rmpv::Value::String("/public".into());
        assert!(!evaluate_starts_with(&actual, &prefix));
    }

    /// AC10: non-string actual returns false (safe deny).
    #[test]
    fn starts_with_non_string_actual_returns_false() {
        let actual = rmpv::Value::Integer(42.into());
        let prefix = rmpv::Value::String("/public".into());
        assert!(!evaluate_starts_with(&actual, &prefix));
    }

    /// AC10: non-string prefix returns false (safe deny).
    #[test]
    fn starts_with_non_string_prefix_returns_false() {
        let actual = rmpv::Value::String("/public/docs".into());
        let prefix = rmpv::Value::Integer(42.into());
        assert!(!evaluate_starts_with(&actual, &prefix));
    }

    /// Case-sensitive: uppercase prefix does not match lowercase actual.
    #[test]
    fn starts_with_case_sensitive() {
        let actual = rmpv::Value::String("/public".into());
        let prefix = rmpv::Value::String("/PUBLIC".into());
        assert!(!evaluate_starts_with(&actual, &prefix));
    }

    // ---- evaluate_ends_with tests ----

    /// AC8: endsWith true for matching suffix.
    #[test]
    fn ends_with_true() {
        let actual = rmpv::Value::String("user@company.com".into());
        let suffix = rmpv::Value::String("@company.com".into());
        assert!(evaluate_ends_with(&actual, &suffix));
    }

    /// AC8: endsWith false for non-matching suffix.
    #[test]
    fn ends_with_false() {
        let actual = rmpv::Value::String("user@other.com".into());
        let suffix = rmpv::Value::String("@company.com".into());
        assert!(!evaluate_ends_with(&actual, &suffix));
    }

    /// AC10: non-string actual returns false (safe deny).
    #[test]
    fn ends_with_non_string_actual_returns_false() {
        let actual = rmpv::Value::Boolean(true);
        let suffix = rmpv::Value::String("@company.com".into());
        assert!(!evaluate_ends_with(&actual, &suffix));
    }

    /// Case-sensitive: uppercase suffix does not match lowercase actual.
    #[test]
    fn ends_with_case_sensitive() {
        let actual = rmpv::Value::String("user@company.com".into());
        let suffix = rmpv::Value::String("@COMPANY.COM".into());
        assert!(!evaluate_ends_with(&actual, &suffix));
    }
}
