//! Base message types shared across all message domains.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/base-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::hlc::{LWWRecord, ORMapRecord};

// ---------------------------------------------------------------------------
// Double-Option helper for nullable + optional fields
// ---------------------------------------------------------------------------

/// Deserializes a field that is both optional (can be absent) and nullable (can be null).
///
/// This maps to the TS pattern `.nullable().optional()`:
/// - Absent field -> `None` (outer Option)
/// - Present field with null -> `Some(None)` (inner Option)
/// - Present field with value -> `Some(Some(value))`
///
/// Without this, serde collapses `null` into the outer `None`, losing the
/// distinction between "field absent" and "field explicitly null".
#[allow(clippy::option_option)]
fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Write concern level defining when an operation is considered acknowledged.
///
/// Maps to `WriteConcernSchema` in `base-schemas.ts`.
/// Variant names use `SCREAMING_CASE` to match TS wire format exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum WriteConcern {
    FIRE_AND_FORGET,
    MEMORY,
    APPLIED,
    REPLICATED,
    PERSISTED,
}

/// Unified change event type for query, search, and cluster subscription updates.
///
/// Maps to `ChangeEventTypeSchema` in `base-schemas.ts`.
/// `SearchUpdateTypeSchema` in `search-schemas.ts` is an alias for this enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeEventType {
    ENTER,
    UPDATE,
    LEAVE,
}

/// Predicate operators for query filtering.
///
/// Maps to `PredicateOpSchema` in `base-schemas.ts`.
/// Lowercase variants match the TS enum values exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PredicateOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    Regex,
    And,
    Or,
    Not,
}

/// Sort direction for query ordering.
///
/// Maps to `z.enum(['asc', 'desc'])` in `QuerySchema.sort` in `base-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// A recursive predicate node for query filtering.
///
/// Maps to `PredicateNodeSchema` in `base-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PredicateNode {
    pub op: PredicateOp,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attribute: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<rmpv::Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub children: Option<Vec<PredicateNode>>,
}

/// Query parameters for filtering, sorting, and pagination.
///
/// Maps to `QuerySchema` in `base-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    /// Key-value filter conditions. `where` is a Rust keyword, so we use raw identifier syntax.
    #[serde(rename = "where")]
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub r#where: Option<HashMap<String, rmpv::Value>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub predicate: Option<PredicateNode>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sort: Option<HashMap<String, SortDirection>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<String>,
}

/// A client operation message containing CRDT data.
///
/// Maps to `ClientOpSchema` in `base-schemas.ts`.
///
/// Note: `Default` produces empty `map_name`/`key` -- for test convenience only.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::option_option)]
pub struct ClientOp {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    pub map_name: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub op_type: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "deserialize_double_option"
    )]
    pub record: Option<Option<LWWRecord<rmpv::Value>>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "deserialize_double_option"
    )]
    pub or_record: Option<Option<ORMapRecord<rmpv::Value>>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "deserialize_double_option"
    )]
    pub or_tag: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub write_concern: Option<WriteConcern>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timeout: Option<u64>,
}

/// Authentication message sent by client to server.
///
/// Maps to `AuthMessageSchema` in `base-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMessage {
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub protocol_version: Option<u32>,
}

/// Authentication required message sent by server to client.
///
/// Maps to `AuthRequiredMessageSchema` in `base-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthRequiredMessage {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::Timestamp;

    /// Helper: round-trip a value through named `MsgPack` serialization.
    fn roundtrip_named<T>(val: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let bytes = rmp_serde::to_vec_named(val).expect("serialize");
        rmp_serde::from_slice(&bytes).expect("deserialize")
    }

    // ---- Enum round-trip tests ----

    #[test]
    fn write_concern_roundtrip() {
        let variants = vec![
            WriteConcern::FIRE_AND_FORGET,
            WriteConcern::MEMORY,
            WriteConcern::APPLIED,
            WriteConcern::REPLICATED,
            WriteConcern::PERSISTED,
        ];
        for v in &variants {
            assert_eq!(&roundtrip_named(v), v);
        }
    }

    #[test]
    fn write_concern_serializes_to_expected_strings() {
        let bytes = rmp_serde::to_vec_named(&WriteConcern::FIRE_AND_FORGET).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "FIRE_AND_FORGET");

        let bytes = rmp_serde::to_vec_named(&WriteConcern::PERSISTED).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "PERSISTED");
    }

    #[test]
    fn change_event_type_roundtrip() {
        let variants = vec![
            ChangeEventType::ENTER,
            ChangeEventType::UPDATE,
            ChangeEventType::LEAVE,
        ];
        for v in &variants {
            assert_eq!(&roundtrip_named(v), v);
        }
    }

    #[test]
    fn change_event_type_serializes_to_expected_strings() {
        let bytes = rmp_serde::to_vec_named(&ChangeEventType::ENTER).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "ENTER");
    }

    #[test]
    fn predicate_op_roundtrip() {
        let variants = vec![
            PredicateOp::Eq,
            PredicateOp::Neq,
            PredicateOp::Gt,
            PredicateOp::Gte,
            PredicateOp::Lt,
            PredicateOp::Lte,
            PredicateOp::Like,
            PredicateOp::Regex,
            PredicateOp::And,
            PredicateOp::Or,
            PredicateOp::Not,
        ];
        for v in &variants {
            assert_eq!(&roundtrip_named(v), v);
        }
    }

    #[test]
    fn predicate_op_serializes_lowercase() {
        let bytes = rmp_serde::to_vec_named(&PredicateOp::Eq).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "eq");

        let bytes = rmp_serde::to_vec_named(&PredicateOp::Gte).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "gte");
    }

    #[test]
    fn sort_direction_roundtrip() {
        let variants = vec![SortDirection::Asc, SortDirection::Desc];
        for v in &variants {
            assert_eq!(&roundtrip_named(v), v);
        }
    }

    #[test]
    fn sort_direction_serializes_lowercase() {
        let bytes = rmp_serde::to_vec_named(&SortDirection::Asc).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "asc");

        let bytes = rmp_serde::to_vec_named(&SortDirection::Desc).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "desc");
    }

    // ---- Struct round-trip tests ----

    #[test]
    fn predicate_node_simple_roundtrip() {
        let node = PredicateNode {
            op: PredicateOp::Eq,
            attribute: Some("name".to_string()),
            value: Some(rmpv::Value::String("Alice".into())),
            children: None,
        };
        assert_eq!(roundtrip_named(&node), node);
    }

    #[test]
    fn predicate_node_recursive_roundtrip() {
        let node = PredicateNode {
            op: PredicateOp::And,
            attribute: None,
            value: None,
            children: Some(vec![
                PredicateNode {
                    op: PredicateOp::Gt,
                    attribute: Some("age".to_string()),
                    value: Some(rmpv::Value::Integer(18.into())),
                    children: None,
                },
                PredicateNode {
                    op: PredicateOp::Eq,
                    attribute: Some("active".to_string()),
                    value: Some(rmpv::Value::Boolean(true)),
                    children: None,
                },
            ]),
        };
        assert_eq!(roundtrip_named(&node), node);
    }

    #[test]
    fn query_full_roundtrip() {
        let mut where_clause = HashMap::new();
        where_clause.insert("status".to_string(), rmpv::Value::String("active".into()));

        let mut sort = HashMap::new();
        sort.insert("createdAt".to_string(), SortDirection::Desc);

        let query = Query {
            r#where: Some(where_clause),
            predicate: Some(PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("type".to_string()),
                value: Some(rmpv::Value::String("user".into())),
                children: None,
            }),
            sort: Some(sort),
            limit: Some(50),
            cursor: Some("abc123".to_string()),
        };
        assert_eq!(roundtrip_named(&query), query);
    }

    #[test]
    fn query_minimal_roundtrip() {
        let query = Query {
            r#where: None,
            predicate: None,
            sort: None,
            limit: None,
            cursor: None,
        };
        assert_eq!(roundtrip_named(&query), query);
    }

    #[test]
    fn client_op_full_roundtrip() {
        let op = ClientOp {
            id: Some("op-1".to_string()),
            map_name: "users".to_string(),
            key: "user-1".to_string(),
            op_type: Some("set".to_string()),
            record: Some(Some(LWWRecord {
                value: Some(rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("name".into()),
                        rmpv::Value::String("Alice".into()),
                    ),
                ])),
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 1,
                    node_id: "node-1".to_string(),
                },
                ttl_ms: Some(60_000),
            })),
            or_record: None,
            or_tag: Some(Some("1700000000000:1:node-1".to_string())),
            write_concern: Some(WriteConcern::APPLIED),
            timeout: Some(5000),
        };
        assert_eq!(roundtrip_named(&op), op);
    }

    #[test]
    fn client_op_minimal_roundtrip() {
        let op = ClientOp {
            id: None,
            map_name: "events".to_string(),
            key: "evt-1".to_string(),
            op_type: None,
            record: None,
            or_record: None,
            or_tag: None,
            write_concern: None,
            timeout: None,
        };
        assert_eq!(roundtrip_named(&op), op);
    }

    #[test]
    fn client_op_nullable_record_null_roundtrip() {
        // record: nullable().optional() -- test the null case (Some(None))
        let op = ClientOp {
            id: None,
            map_name: "test".to_string(),
            key: "k".to_string(),
            op_type: None,
            record: Some(None),
            or_record: Some(None),
            or_tag: Some(None),
            write_concern: None,
            timeout: None,
        };
        assert_eq!(roundtrip_named(&op), op);
    }

    #[test]
    fn auth_message_roundtrip() {
        let msg = AuthMessage {
            token: "jwt-token-here".to_string(),
            protocol_version: Some(1),
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn auth_message_without_version_roundtrip() {
        let msg = AuthMessage {
            token: "some-token".to_string(),
            protocol_version: None,
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn auth_required_message_roundtrip() {
        let msg = AuthRequiredMessage {};
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- camelCase field name verification ----

    #[test]
    fn auth_message_no_type_field() {
        // The type discriminator is owned by the Message enum, not by inner structs.
        // Verify AuthMessage no longer serializes a "type" key.
        let msg = AuthMessage {
            token: "t".to_string(),
            protocol_version: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = val.as_map().expect("should be a map");

        let has_type_key = map.iter().any(|(k, _)| k.as_str() == Some("type"));
        assert!(!has_type_key, "inner struct must not have a 'type' field");
    }

    #[test]
    fn query_where_field_serializes_as_where() {
        let mut w = HashMap::new();
        w.insert("x".to_string(), rmpv::Value::Integer(1.into()));
        let query = Query {
            r#where: Some(w),
            predicate: None,
            sort: None,
            limit: None,
            cursor: None,
        };
        let bytes = rmp_serde::to_vec_named(&query).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = val.as_map().expect("should be a map");

        let has_where_key = map.iter().any(|(k, _)| k.as_str() == Some("where"));
        assert!(has_where_key, "expected 'where' field key in serialized output");
    }

    #[test]
    fn client_op_camel_case_field_names() {
        let op = ClientOp {
            id: None,
            map_name: "test".to_string(),
            key: "k".to_string(),
            op_type: Some("set".to_string()),
            record: None,
            or_record: None,
            or_tag: None,
            write_concern: Some(WriteConcern::MEMORY),
            timeout: None,
        };
        let bytes = rmp_serde::to_vec_named(&op).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = val.as_map().expect("should be a map");

        let keys: Vec<&str> = map
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect();

        assert!(keys.contains(&"mapName"), "expected camelCase 'mapName'");
        assert!(keys.contains(&"opType"), "expected camelCase 'opType'");
        assert!(keys.contains(&"writeConcern"), "expected camelCase 'writeConcern'");
    }

    // ---- Timestamp camelCase verification (AC-5) ----

    #[test]
    fn timestamp_to_vec_named_camel_case() {
        let ts = Timestamp {
            millis: 1_700_000_000_000,
            counter: 42,
            node_id: "node-1".to_string(),
        };
        let bytes = rmp_serde::to_vec_named(&ts).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = val.as_map().expect("should be a map");

        let keys: Vec<&str> = map
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect();

        assert!(keys.contains(&"nodeId"), "expected camelCase 'nodeId', got: {keys:?}");
        assert!(keys.contains(&"millis"), "expected 'millis'");
        assert!(keys.contains(&"counter"), "expected 'counter'");
    }

    #[test]
    fn lww_record_to_vec_named_camel_case() {
        let record: LWWRecord<rmpv::Value> = LWWRecord {
            value: Some(rmpv::Value::String("test".into())),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "n".to_string(),
            },
            ttl_ms: Some(5000),
        };
        let bytes = rmp_serde::to_vec_named(&record).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = val.as_map().expect("should be a map");

        let keys: Vec<&str> = map
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect();

        assert!(keys.contains(&"ttlMs"), "expected camelCase 'ttlMs', got: {keys:?}");
    }

    #[test]
    fn or_map_record_to_vec_named_camel_case() {
        let record: ORMapRecord<rmpv::Value> = ORMapRecord {
            value: rmpv::Value::Integer(42.into()),
            timestamp: Timestamp {
                millis: 100,
                counter: 0,
                node_id: "n".to_string(),
            },
            tag: "100:0:n".to_string(),
            ttl_ms: Some(3000),
        };
        let bytes = rmp_serde::to_vec_named(&record).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = val.as_map().expect("should be a map");

        let keys: Vec<&str> = map
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect();

        assert!(keys.contains(&"ttlMs"), "expected camelCase 'ttlMs', got: {keys:?}");
    }

    // ---- LWWRecord<rmpv::Value> round-trip (AC-lww-rmpv-roundtrip) ----

    #[test]
    fn lww_record_rmpv_value_roundtrip() {
        let record: LWWRecord<rmpv::Value> = LWWRecord {
            value: Some(rmpv::Value::Map(vec![
                (
                    rmpv::Value::String("name".into()),
                    rmpv::Value::String("Alice".into()),
                ),
                (
                    rmpv::Value::String("age".into()),
                    rmpv::Value::Integer(30.into()),
                ),
                (
                    rmpv::Value::String("tags".into()),
                    rmpv::Value::Array(vec![
                        rmpv::Value::String("admin".into()),
                        rmpv::Value::String("active".into()),
                    ]),
                ),
            ])),
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 7,
                node_id: "node-xyz".to_string(),
            },
            ttl_ms: Some(30_000),
        };
        let bytes = rmp_serde::to_vec_named(&record).unwrap();
        let decoded: LWWRecord<rmpv::Value> = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(record, decoded);
    }

    #[test]
    fn lww_record_rmpv_value_tombstone_roundtrip() {
        let record: LWWRecord<rmpv::Value> = LWWRecord {
            value: None,
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 0,
                node_id: "node-1".to_string(),
            },
            ttl_ms: None,
        };
        let bytes = rmp_serde::to_vec_named(&record).unwrap();
        let decoded: LWWRecord<rmpv::Value> = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(record, decoded);
    }

    // ---- Default derive tests ----

    #[test]
    fn query_default_constructs_all_none() {
        let q = Query::default();
        assert_eq!(q.r#where, None);
        assert_eq!(q.predicate, None);
        assert_eq!(q.sort, None);
        assert_eq!(q.limit, None);
        assert_eq!(q.cursor, None);
    }

    #[test]
    fn client_op_default_constructs_with_empty_required_fields() {
        let op = ClientOp::default();
        assert_eq!(op.map_name, "");
        assert_eq!(op.key, "");
        assert_eq!(op.timeout, None);
        assert_eq!(op.write_concern, None);
    }
}
