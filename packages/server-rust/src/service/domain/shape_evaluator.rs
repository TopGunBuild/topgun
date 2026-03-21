//! Shape evaluator module providing pure-function shape evaluation.
//!
//! Evaluates `SyncShape` filters against `rmpv::Value` records using the
//! existing `evaluate_predicate()` function from the predicate engine.
//! Also provides field projection for partial replication.

use topgun_core::schema::SyncShape;

use super::predicate::evaluate_predicate;

// ---------------------------------------------------------------------------
// Shape evaluation functions
// ---------------------------------------------------------------------------

/// Evaluates whether a record matches a shape's filter.
///
/// If the shape has no filter (`filter` is `None`), the record always matches.
/// Otherwise delegates to `evaluate_predicate()` for `PredicateNode` evaluation.
#[must_use]
pub fn matches(shape: &SyncShape, record: &rmpv::Value) -> bool {
    match &shape.filter {
        None => true,
        Some(predicate) => evaluate_predicate(predicate, record),
    }
}

/// Projects a record to include only the specified fields.
///
/// Strips non-projected fields from a Map value, returning the projected subset.
/// If the record is not a Map, returns it unchanged (cloned).
#[must_use]
pub fn project(fields: &[String], record: &rmpv::Value) -> rmpv::Value {
    let Some(map) = record.as_map() else {
        return record.clone();
    };

    let projected: Vec<(rmpv::Value, rmpv::Value)> = map
        .iter()
        .filter(|(k, _)| {
            k.as_str()
                .is_some_and(|key_str| fields.iter().any(|f| f == key_str))
        })
        .cloned()
        .collect();

    rmpv::Value::Map(projected)
}

/// Combines match + project: returns `None` if filtered out, `Some(projected_value)` if matching.
///
/// If `fields` is `None` on the shape, no projection is applied and the full
/// record value is returned. Neither `matches()` nor `project()` uses the
/// record key, so callers building a `ShapeRecord` pass the key independently.
#[must_use]
pub fn apply_shape(shape: &SyncShape, record: &rmpv::Value) -> Option<rmpv::Value> {
    if !matches(shape, record) {
        return None;
    }

    match &shape.fields {
        None => Some(record.clone()),
        Some(fields) => Some(project(fields, record)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::base::{PredicateNode, PredicateOp};

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

    /// Helper: build a SyncShape with given filter and fields.
    fn make_shape(
        filter: Option<PredicateNode>,
        fields: Option<Vec<String>>,
    ) -> SyncShape {
        SyncShape {
            shape_id: "s1".into(),
            map_name: "users".into(),
            filter,
            fields,
            limit: None,
        }
    }

    // ---- matches tests ----

    #[test]
    fn matches_none_filter_returns_true() {
        let shape = make_shape(None, None);
        let record = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        assert!(matches(&shape, &record));
    }

    #[test]
    fn matches_eq_filter_matching_record() {
        let shape = make_shape(
            Some(leaf(PredicateOp::Eq, "status", rmpv::Value::String("active".into()))),
            None,
        );
        let record = make_map(vec![("status", rmpv::Value::String("active".into()))]);
        assert!(matches(&shape, &record));
    }

    #[test]
    fn matches_eq_filter_non_matching_record() {
        let shape = make_shape(
            Some(leaf(PredicateOp::Eq, "status", rmpv::Value::String("active".into()))),
            None,
        );
        let record = make_map(vec![("status", rmpv::Value::String("inactive".into()))]);
        assert!(!matches(&shape, &record));
    }

    #[test]
    fn matches_and_compound_filter() {
        let shape = make_shape(
            Some(combinator(
                PredicateOp::And,
                vec![
                    leaf(PredicateOp::Gte, "age", rmpv::Value::Integer(18.into())),
                    leaf(PredicateOp::Eq, "active", rmpv::Value::Boolean(true)),
                ],
            )),
            None,
        );

        let matching = make_map(vec![
            ("age", rmpv::Value::Integer(25.into())),
            ("active", rmpv::Value::Boolean(true)),
        ]);
        assert!(matches(&shape, &matching));

        let non_matching = make_map(vec![
            ("age", rmpv::Value::Integer(15.into())),
            ("active", rmpv::Value::Boolean(true)),
        ]);
        assert!(!matches(&shape, &non_matching));
    }

    // ---- project tests ----

    #[test]
    fn project_strips_non_projected_fields() {
        let record = make_map(vec![
            ("name", rmpv::Value::String("Alice".into())),
            ("age", rmpv::Value::Integer(30.into())),
            ("email", rmpv::Value::String("alice@test.com".into())),
        ]);
        let fields = vec!["name".to_string(), "age".to_string()];
        let result = project(&fields, &record);

        let expected = make_map(vec![
            ("name", rmpv::Value::String("Alice".into())),
            ("age", rmpv::Value::Integer(30.into())),
        ]);
        assert_eq!(result, expected);
    }

    #[test]
    fn project_non_map_returns_unchanged() {
        let record = rmpv::Value::String("not a map".into());
        let fields = vec!["name".to_string()];
        let result = project(&fields, &record);
        assert_eq!(result, record);
    }

    // ---- apply_shape tests ----

    #[test]
    fn apply_shape_returns_none_for_non_matching() {
        let shape = make_shape(
            Some(leaf(PredicateOp::Eq, "status", rmpv::Value::String("active".into()))),
            None,
        );
        let record = make_map(vec![("status", rmpv::Value::String("inactive".into()))]);
        assert!(apply_shape(&shape, &record).is_none());
    }

    #[test]
    fn apply_shape_returns_projected_value_for_matching() {
        let shape = make_shape(
            Some(leaf(PredicateOp::Eq, "status", rmpv::Value::String("active".into()))),
            Some(vec!["name".to_string()]),
        );
        let record = make_map(vec![
            ("name", rmpv::Value::String("Alice".into())),
            ("status", rmpv::Value::String("active".into())),
            ("email", rmpv::Value::String("alice@test.com".into())),
        ]);

        let result = apply_shape(&shape, &record);
        let expected = make_map(vec![("name", rmpv::Value::String("Alice".into()))]);
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn apply_shape_fields_none_returns_full_value() {
        let shape = make_shape(None, None);
        let record = make_map(vec![
            ("name", rmpv::Value::String("Alice".into())),
            ("age", rmpv::Value::Integer(30.into())),
        ]);

        let result = apply_shape(&shape, &record);
        assert_eq!(result, Some(record));
    }
}
