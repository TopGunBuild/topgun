//! Transport-neutral keyset cursor for paginated queries.
//!
//! A single cursor implementation consumed by both the HTTP sync handler and (in the
//! DAG Cursor stage) the structured query pipeline. Keeping one copy here prevents
//! the HTTP-specific `HttpCursorData` and any future per-transport cursors from
//! diverging silently.

use topgun_core::messages::base::SortDirection;

// ---------------------------------------------------------------------------
// CursorData
// ---------------------------------------------------------------------------

/// Keyset cursor that encodes the resume point for paginated queries.
///
/// Multi-field keyset: `sort_values` holds one entry per ordered sort field (aligned to
/// `Query.sort`). A single-field cursor is the degenerate 1-element list, maintaining
/// backward compatibility with the former single-field HTTP cursor.
///
/// The cursor is JSON-serialized and base64url-encoded for safe transport in HTTP
/// headers and URL query parameters.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorData {
    /// One `(field_name, last_value)` pair per ordered sort field.
    ///
    /// Aligned to the caller's `Query.sort` order. For key-only ordering, this list
    /// is empty and the `last_key` tie-break is the sole comparator.
    pub sort_values: Vec<SortValue>,
    /// Key of the last result in the page, used as tiebreaker for equal sort values.
    pub last_key: String,
    /// Hash of the predicate applied in this query (0 if no predicate).
    ///
    /// Stored as `u64` because predicate hashes are unsigned integer values.
    pub predicate_hash: u64,
    /// Hash of the sort specification (0 if no sort).
    ///
    /// Stored as `u64` because sort hashes are unsigned integer values.
    pub sort_hash: u64,
    /// Unix timestamp (ms) when this cursor was created; used for expiry checks.
    ///
    /// `i64` to match `Timestamp.millis` (system clock returns signed ms since epoch).
    pub timestamp: i64,
}

/// One sort-field position in the multi-field keyset tuple.
///
/// Pairs the field name with the last seen value for that field and the sort direction
/// used for this query, so `is_after_cursor` can apply per-field ASC/DESC semantics
/// without needing the original `Query.sort` list at comparison time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortValue {
    /// Name of the sort field.
    pub field: String,
    /// Last seen value for this field, encoded as JSON for cursor serialization.
    ///
    /// `serde_json::Value` is the serialization-friendly representation; the store
    /// returns `rmpv::Value` which is converted via [`rmpv_to_json_value`] before
    /// storing in the cursor.
    pub value: serde_json::Value,
    /// Sort direction for this field.
    pub direction: SortDirection,
}

// ---------------------------------------------------------------------------
// Encode / decode
// ---------------------------------------------------------------------------

/// Encodes cursor data as a base64url-encoded JSON string for HTTP transport.
///
/// The encoding is URL-safe with no padding so the cursor can be passed as a URL
/// query parameter without additional escaping.
///
/// # Panics
///
/// Panics if `serde_json` fails to serialize `CursorData`, which cannot happen for
/// well-formed `CursorData` values (all fields are JSON-serializable primitives).
#[must_use]
pub fn encode_cursor(data: &CursorData) -> String {
    let json = serde_json::to_vec(data).expect("CursorData serialization is infallible");
    base64::engine::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &json)
}

/// Decodes and validates a cursor string.
///
/// Returns `None` when the cursor is malformed, not valid base64url, or fails JSON
/// deserialization. Callers must additionally validate the timestamp for expiry and
/// check `predicate_hash`/`sort_hash` against the current query.
#[must_use]
pub fn decode_cursor(cursor: &str) -> Option<CursorData> {
    let bytes =
        base64::engine::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, cursor)
            .ok()?;
    serde_json::from_slice::<CursorData>(&bytes).ok()
}

// ---------------------------------------------------------------------------
// Cursor position check
// ---------------------------------------------------------------------------

/// TTL for cursors: cursors older than 10 minutes are rejected.
const CURSOR_TTL_MS: i64 = 10 * 60 * 1000;

/// Returns `true` when `(key, record_value)` comes strictly **after** the cursor
/// position, meaning it should appear on the next page.
///
/// Multi-field keyset semantics:
/// - Walk `cursor.sort_values` in order. For each field:
///   - Extract the field's value from `record_value` (an `rmpv::Value::Map`).
///   - Compare it to the cursor's stored `json_value` using `compare_rmpv_to_json`.
///   - ASC: `record_val > cursor_val` → include; `record_val < cursor_val` → exclude;
///     equal → continue to next field.
///   - DESC: `record_val < cursor_val` → include; `record_val > cursor_val` → exclude;
///     equal → continue to next field.
/// - After all sort fields are equal, apply the `last_key` tie-break: include only
///   when `key > cursor.last_key`.
/// - When `sort_values` is empty (key-only ordering), only the tie-break applies.
#[must_use]
pub fn is_after_cursor(key: &str, record_value: &rmpv::Value, cursor: &CursorData) -> bool {
    for sv in &cursor.sort_values {
        // Extract field value from the rmpv map record.
        let field_val: &rmpv::Value = match record_value {
            rmpv::Value::Map(pairs) => pairs
                .iter()
                .find(|(k, _)| k.as_str() == Some(sv.field.as_str()))
                .map_or(&rmpv::Value::Nil, |(_, v)| v),
            _ => &rmpv::Value::Nil,
        };

        let cmp = compare_rmpv_to_json(field_val, &sv.value);

        match sv.direction {
            SortDirection::Asc => {
                if cmp > 0 {
                    return true; // strictly after on this field
                }
                if cmp < 0 {
                    return false; // strictly before on this field
                }
                // Equal: continue to next field
            }
            SortDirection::Desc => {
                if cmp < 0 {
                    return true; // strictly after (lower value) in descending order
                }
                if cmp > 0 {
                    return false; // strictly before (higher value) in descending order
                }
                // Equal: continue to next field
            }
        }
    }

    // All sort fields are equal (or sort_values is empty): apply key tie-break.
    // Keys are unique strings; the tie-break is always ascending.
    key > cursor.last_key.as_str()
}

/// Validates cursor authenticity against the current query's hashes.
///
/// Returns `false` when the `predicate_hash` or `sort_hash` in the cursor does not
/// match the supplied values, indicating a cursor was produced by a different query
/// shape. Callers should reject the request with a 400 when this returns `false`.
#[must_use]
pub fn validate_cursor_hashes(cursor: &CursorData, predicate_hash: u64, sort_hash: u64) -> bool {
    cursor.predicate_hash == predicate_hash && cursor.sort_hash == sort_hash
}

/// Validates that the cursor has not expired relative to `now_ms`.
///
/// Returns `false` when the cursor is older than [`CURSOR_TTL_MS`].
#[must_use]
pub fn validate_cursor_expiry(cursor: &CursorData, now_ms: i64) -> bool {
    now_ms - cursor.timestamp <= CURSOR_TTL_MS
}

// ---------------------------------------------------------------------------
// rmpv / JSON comparison helper
// ---------------------------------------------------------------------------

/// Compares an `rmpv::Value` (from the record store) to a `serde_json::Value`
/// (from cursor JSON).
///
/// Returns negative/zero/positive as a standard three-way comparison.
/// `Nil`/`null` sorts last. Strings are compared lexicographically. Numbers are
/// compared as `f64`. Mixed types compare by type-tag string for stable ordering.
pub fn compare_rmpv_to_json(rmpv_val: &rmpv::Value, json_val: &serde_json::Value) -> i32 {
    match (rmpv_val, json_val) {
        (rmpv::Value::Nil, serde_json::Value::Null) => 0,
        (rmpv::Value::Nil, _) => 1, // nil sorts after any non-null value
        (_, serde_json::Value::Null) => -1, // any non-nil sorts before null

        (rmpv::Value::String(s), serde_json::Value::String(js)) => {
            s.as_str().unwrap_or("").cmp(js.as_str()).into_i32_sign()
        }
        (rmpv::Value::Integer(i), serde_json::Value::Number(n)) => {
            let a = i.as_f64().unwrap_or(f64::NAN);
            let b = n.as_f64().unwrap_or(f64::NAN);
            a.partial_cmp(&b).map_or(0, OrderingExt::into_i32_sign)
        }
        (rmpv::Value::F32(a), serde_json::Value::Number(n)) => {
            let b = n.as_f64().unwrap_or(f64::NAN);
            f64::from(*a)
                .partial_cmp(&b)
                .map_or(0, OrderingExt::into_i32_sign)
        }
        (rmpv::Value::F64(a), serde_json::Value::Number(n)) => {
            let b = n.as_f64().unwrap_or(f64::NAN);
            a.partial_cmp(&b).map_or(0, OrderingExt::into_i32_sign)
        }
        // Type mismatch: compare by type-tag string for stable ordering across types.
        _ => {
            let a_tag = rmpv_type_tag(rmpv_val);
            let b_tag = json_type_tag(json_val);
            a_tag.cmp(b_tag).into_i32_sign()
        }
    }
}

fn rmpv_type_tag(v: &rmpv::Value) -> &'static str {
    match v {
        rmpv::Value::Nil => "nil",
        rmpv::Value::Boolean(_) => "bool",
        rmpv::Value::Integer(_) | rmpv::Value::F32(_) | rmpv::Value::F64(_) => "number",
        rmpv::Value::String(_) => "string",
        rmpv::Value::Binary(_) => "binary",
        rmpv::Value::Array(_) => "array",
        rmpv::Value::Map(_) => "map",
        rmpv::Value::Ext(_, _) => "ext",
    }
}

fn json_type_tag(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "nil",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "map",
    }
}

/// Converts an `rmpv::Value` to a `serde_json::Value` for cursor serialization.
///
/// Only primitive types (nil, bool, integer, float, string) are converted. Complex
/// types (map, array, binary, ext) return `None` because they are not meaningfully
/// sortable as cursor positions.
pub fn rmpv_to_json_value(v: &rmpv::Value) -> Option<serde_json::Value> {
    match v {
        rmpv::Value::Nil => Some(serde_json::Value::Null),
        rmpv::Value::Boolean(b) => Some(serde_json::Value::Bool(*b)),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Some(serde_json::Value::Number(serde_json::Number::from(n)))
            } else {
                i.as_u64()
                    .map(|n| serde_json::Value::Number(serde_json::Number::from(n)))
            }
        }
        rmpv::Value::F32(f) => {
            serde_json::Number::from_f64(f64::from(*f)).map(serde_json::Value::Number)
        }
        rmpv::Value::F64(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number),
        rmpv::Value::String(s) => s.as_str().map(|s| serde_json::Value::String(s.to_owned())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// OrderingExt helper
// ---------------------------------------------------------------------------

trait OrderingExt {
    fn into_i32_sign(self) -> i32;
}

impl OrderingExt for std::cmp::Ordering {
    fn into_i32_sign(self) -> i32 {
        match self {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // encode / decode round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn encode_decode_roundtrip_single_field() {
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::Value::Number(serde_json::Number::from(42i64)),
                direction: SortDirection::Asc,
            }],
            last_key: "record-abc".to_string(),
            predicate_hash: 12345u64,
            sort_hash: 67890u64,
            timestamp: 1_700_000_000_000i64,
        };

        let encoded = encode_cursor(&cursor);
        let decoded = decode_cursor(&encoded).expect("valid cursor must decode");

        assert_eq!(decoded.last_key, cursor.last_key);
        assert_eq!(decoded.predicate_hash, cursor.predicate_hash);
        assert_eq!(decoded.sort_hash, cursor.sort_hash);
        assert_eq!(decoded.timestamp, cursor.timestamp);
        assert_eq!(decoded.sort_values.len(), 1);
        assert_eq!(decoded.sort_values[0].field, "score");
        assert_eq!(decoded.sort_values[0].direction, SortDirection::Asc);
        assert_eq!(
            decoded.sort_values[0].value,
            serde_json::Value::Number(serde_json::Number::from(42i64))
        );
    }

    #[test]
    fn encode_decode_roundtrip_multi_field() {
        let cursor = CursorData {
            sort_values: vec![
                SortValue {
                    field: "age".to_string(),
                    value: serde_json::Value::Number(serde_json::Number::from(30i64)),
                    direction: SortDirection::Asc,
                },
                SortValue {
                    field: "name".to_string(),
                    value: serde_json::json!("Alice"),
                    direction: SortDirection::Desc,
                },
            ],
            last_key: "user-42".to_string(),
            predicate_hash: 0u64,
            sort_hash: 99u64,
            timestamp: 1_700_000_000_001i64,
        };

        let encoded = encode_cursor(&cursor);
        let decoded = decode_cursor(&encoded).expect("multi-field cursor must decode");

        assert_eq!(decoded.sort_values.len(), 2);
        assert_eq!(decoded.sort_values[0].field, "age");
        assert_eq!(decoded.sort_values[0].direction, SortDirection::Asc);
        assert_eq!(decoded.sort_values[1].field, "name");
        assert_eq!(decoded.sort_values[1].direction, SortDirection::Desc);
        assert_eq!(decoded.last_key, "user-42");
    }

    #[test]
    fn decode_rejects_invalid_inputs() {
        assert!(decode_cursor("!!!not-base64!!!").is_none());
        assert!(decode_cursor("aGVsbG8=").is_none()); // valid base64, not a cursor
    }

    // -----------------------------------------------------------------------
    // is_after_cursor: single-field ASC
    // -----------------------------------------------------------------------

    fn make_record(field: &str, val: rmpv::Value) -> rmpv::Value {
        rmpv::Value::Map(vec![(rmpv::Value::String(field.into()), val)])
    }

    #[test]
    fn is_after_cursor_asc_greater_value_returns_true() {
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::Value::Number(serde_json::Number::from(10i64)),
                direction: SortDirection::Asc,
            }],
            last_key: "k".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };
        let record = make_record("score", rmpv::Value::Integer(20.into()));
        assert!(is_after_cursor("z", &record, &cursor));
    }

    #[test]
    fn is_after_cursor_asc_lesser_value_returns_false() {
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::Value::Number(serde_json::Number::from(10i64)),
                direction: SortDirection::Asc,
            }],
            last_key: "k".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };
        let record = make_record("score", rmpv::Value::Integer(5.into()));
        assert!(!is_after_cursor("a", &record, &cursor));
    }

    #[test]
    fn is_after_cursor_asc_equal_value_key_tiebreak() {
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::Value::Number(serde_json::Number::from(10i64)),
                direction: SortDirection::Asc,
            }],
            last_key: "m".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };
        let record = make_record("score", rmpv::Value::Integer(10.into()));
        // key "z" > "m" → after cursor
        assert!(is_after_cursor("z", &record, &cursor));
        // key "a" < "m" → before cursor
        assert!(!is_after_cursor("a", &record, &cursor));
        // key "m" == "m" → not after cursor (strictly after)
        assert!(!is_after_cursor("m", &record, &cursor));
    }

    // -----------------------------------------------------------------------
    // is_after_cursor: single-field DESC
    // -----------------------------------------------------------------------

    #[test]
    fn is_after_cursor_desc_lower_value_returns_true() {
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::Value::Number(serde_json::Number::from(10i64)),
                direction: SortDirection::Desc,
            }],
            last_key: "k".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };
        let record = make_record("score", rmpv::Value::Integer(5.into()));
        assert!(is_after_cursor("z", &record, &cursor));
    }

    #[test]
    fn is_after_cursor_desc_higher_value_returns_false() {
        let cursor = CursorData {
            sort_values: vec![SortValue {
                field: "score".to_string(),
                value: serde_json::Value::Number(serde_json::Number::from(10i64)),
                direction: SortDirection::Desc,
            }],
            last_key: "k".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };
        let record = make_record("score", rmpv::Value::Integer(20.into()));
        assert!(!is_after_cursor("z", &record, &cursor));
    }

    // -----------------------------------------------------------------------
    // is_after_cursor: multi-field (ASC + DESC mixed)
    // -----------------------------------------------------------------------

    fn make_two_field_record(f1: &str, v1: rmpv::Value, f2: &str, v2: rmpv::Value) -> rmpv::Value {
        rmpv::Value::Map(vec![
            (rmpv::Value::String(f1.into()), v1),
            (rmpv::Value::String(f2.into()), v2),
        ])
    }

    #[test]
    fn is_after_cursor_multi_field_first_field_dominates() {
        // Cursor: age ASC at 30, name DESC at "Alice", last_key "k1"
        let cursor = CursorData {
            sort_values: vec![
                SortValue {
                    field: "age".to_string(),
                    value: serde_json::Value::Number(serde_json::Number::from(30i64)),
                    direction: SortDirection::Asc,
                },
                SortValue {
                    field: "name".to_string(),
                    value: serde_json::json!("Alice"),
                    direction: SortDirection::Desc,
                },
            ],
            last_key: "k1".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };

        // Record with age=40 (> 30 ASC) → after cursor regardless of name
        let rec_after = make_two_field_record(
            "age",
            rmpv::Value::Integer(40.into()),
            "name",
            rmpv::Value::String("Zara".into()),
        );
        assert!(is_after_cursor("z", &rec_after, &cursor));

        // Record with age=20 (< 30 ASC) → before cursor regardless of name
        let rec_before = make_two_field_record(
            "age",
            rmpv::Value::Integer(20.into()),
            "name",
            rmpv::Value::String("Zara".into()),
        );
        assert!(!is_after_cursor("z", &rec_before, &cursor));
    }

    #[test]
    fn is_after_cursor_multi_field_second_field_tiebreak() {
        // Cursor: age ASC at 30, name DESC at "Alice", last_key "k1"
        let cursor = CursorData {
            sort_values: vec![
                SortValue {
                    field: "age".to_string(),
                    value: serde_json::Value::Number(serde_json::Number::from(30i64)),
                    direction: SortDirection::Asc,
                },
                SortValue {
                    field: "name".to_string(),
                    value: serde_json::json!("Alice"),
                    direction: SortDirection::Desc,
                },
            ],
            last_key: "k1".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };

        // age equal (30), name "Aardvark" < "Alice" → DESC means lower value is after
        let rec_aardvark = make_two_field_record(
            "age",
            rmpv::Value::Integer(30.into()),
            "name",
            rmpv::Value::String("Aardvark".into()),
        );
        assert!(is_after_cursor("z", &rec_aardvark, &cursor));

        // age equal (30), name "Zara" > "Alice" → DESC means higher value is before
        let rec_zara = make_two_field_record(
            "age",
            rmpv::Value::Integer(30.into()),
            "name",
            rmpv::Value::String("Zara".into()),
        );
        assert!(!is_after_cursor("z", &rec_zara, &cursor));
    }

    #[test]
    fn is_after_cursor_empty_sort_values_key_only() {
        // No sort fields → key-only ordering.
        let cursor = CursorData {
            sort_values: vec![],
            last_key: "m".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: 0,
        };
        let record = rmpv::Value::Nil;
        assert!(is_after_cursor("z", &record, &cursor));
        assert!(!is_after_cursor("a", &record, &cursor));
        assert!(!is_after_cursor("m", &record, &cursor));
    }

    // -----------------------------------------------------------------------
    // validate_cursor_hashes
    // -----------------------------------------------------------------------

    #[test]
    fn validate_cursor_hashes_match() {
        let cursor = CursorData {
            sort_values: vec![],
            last_key: "k".to_string(),
            predicate_hash: 42u64,
            sort_hash: 99u64,
            timestamp: 0,
        };
        assert!(validate_cursor_hashes(&cursor, 42, 99));
        assert!(!validate_cursor_hashes(&cursor, 43, 99)); // predicate mismatch
        assert!(!validate_cursor_hashes(&cursor, 42, 100)); // sort mismatch
        assert!(!validate_cursor_hashes(&cursor, 1, 2)); // both mismatch
    }

    // -----------------------------------------------------------------------
    // validate_cursor_expiry
    // -----------------------------------------------------------------------

    #[test]
    fn validate_cursor_expiry_within_ttl() {
        let now_ms = 1_700_000_000_000i64;
        let cursor = CursorData {
            sort_values: vec![],
            last_key: "k".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: now_ms - 60_000, // 1 minute ago → within TTL
        };
        assert!(validate_cursor_expiry(&cursor, now_ms));
    }

    #[test]
    fn validate_cursor_expiry_past_ttl() {
        let now_ms = 1_700_000_000_000i64;
        let cursor = CursorData {
            sort_values: vec![],
            last_key: "k".to_string(),
            predicate_hash: 0,
            sort_hash: 0,
            timestamp: now_ms - 11 * 60 * 1000, // 11 minutes ago → expired
        };
        assert!(!validate_cursor_expiry(&cursor, now_ms));
    }

    // -----------------------------------------------------------------------
    // compare_rmpv_to_json
    // -----------------------------------------------------------------------

    #[test]
    fn compare_nil_null_equal() {
        assert_eq!(
            compare_rmpv_to_json(&rmpv::Value::Nil, &serde_json::Value::Null),
            0
        );
    }

    #[test]
    fn compare_nil_sorts_after_non_null() {
        // nil > any non-null
        assert!(compare_rmpv_to_json(&rmpv::Value::Nil, &serde_json::json!(1)) > 0);
    }

    #[test]
    fn compare_non_nil_sorts_before_null() {
        // any non-nil < null
        assert!(
            compare_rmpv_to_json(&rmpv::Value::Integer(5.into()), &serde_json::Value::Null) < 0
        );
    }

    #[test]
    fn compare_integers() {
        assert_eq!(
            compare_rmpv_to_json(&rmpv::Value::Integer(10.into()), &serde_json::json!(10)),
            0
        );
        assert!(compare_rmpv_to_json(&rmpv::Value::Integer(11.into()), &serde_json::json!(10)) > 0);
        assert!(compare_rmpv_to_json(&rmpv::Value::Integer(9.into()), &serde_json::json!(10)) < 0);
    }

    #[test]
    fn compare_strings() {
        assert_eq!(
            compare_rmpv_to_json(
                &rmpv::Value::String("abc".into()),
                &serde_json::json!("abc")
            ),
            0
        );
        assert!(
            compare_rmpv_to_json(&rmpv::Value::String("b".into()), &serde_json::json!("a")) > 0
        );
    }
}

// ---------------------------------------------------------------------------
// Proptest suite
// ---------------------------------------------------------------------------

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// encode_cursor → decode_cursor must yield an equal cursor for any valid input.
        #[test]
        fn roundtrip_any_cursor(
            last_key in "[a-z]{1,20}",
            predicate_hash in 0u64..u64::MAX,
            sort_hash in 0u64..u64::MAX,
            timestamp in 0i64..2_000_000_000_000i64,
            num_sort_fields in 0usize..4usize,
        ) {
            // Build a cursor with `num_sort_fields` fields.
            let sort_values: Vec<SortValue> = (0..num_sort_fields)
                .map(|i| {
                    let i_i64 = i64::try_from(i).unwrap_or(0);
                    SortValue {
                        field: format!("field{i}"),
                        value: serde_json::Value::Number(serde_json::Number::from(i_i64)),
                        direction: if i % 2 == 0 { SortDirection::Asc } else { SortDirection::Desc },
                    }
                })
                .collect();

            let cursor = CursorData {
                sort_values,
                last_key: last_key.clone(),
                predicate_hash,
                sort_hash,
                timestamp,
            };

            let encoded = encode_cursor(&cursor);
            let decoded = decode_cursor(&encoded).expect("roundtrip must succeed");

            prop_assert_eq!(decoded.last_key, last_key);
            prop_assert_eq!(decoded.predicate_hash, predicate_hash);
            prop_assert_eq!(decoded.sort_hash, sort_hash);
            prop_assert_eq!(decoded.timestamp, timestamp);
            prop_assert_eq!(decoded.sort_values.len(), num_sort_fields);
        }

        /// is_after_cursor with a single ASC field: a strictly greater value is always after.
        #[test]
        fn is_after_asc_greater_always_true(
            cursor_val in 0i64..1000i64,
            record_val in 1001i64..2000i64,
            last_key in "[a-m]{1,10}",
            test_key in "[n-z]{1,10}",
        ) {
            let cursor = CursorData {
                sort_values: vec![SortValue {
                    field: "v".to_string(),
                    value: serde_json::Value::Number(serde_json::Number::from(cursor_val)),
                    direction: SortDirection::Asc,
                }],
                last_key,
                predicate_hash: 0,
                sort_hash: 0,
                timestamp: 0,
            };
            let record = rmpv::Value::Map(vec![(
                rmpv::Value::String("v".into()),
                rmpv::Value::Integer(record_val.into()),
            )]);
            prop_assert!(is_after_cursor(&test_key, &record, &cursor));
        }

        /// is_after_cursor with a single DESC field: a strictly lower value is always after.
        #[test]
        fn is_after_desc_lower_always_true(
            cursor_val in 1001i64..2000i64,
            record_val in 0i64..1000i64,
            last_key in "[a-m]{1,10}",
            test_key in "[n-z]{1,10}",
        ) {
            let cursor = CursorData {
                sort_values: vec![SortValue {
                    field: "v".to_string(),
                    value: serde_json::Value::Number(serde_json::Number::from(cursor_val)),
                    direction: SortDirection::Desc,
                }],
                last_key,
                predicate_hash: 0,
                sort_hash: 0,
                timestamp: 0,
            };
            let record = rmpv::Value::Map(vec![(
                rmpv::Value::String("v".into()),
                rmpv::Value::Integer(record_val.into()),
            )]);
            prop_assert!(is_after_cursor(&test_key, &record, &cursor));
        }
    }
}
