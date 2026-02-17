//! Query domain message types for subscription lifecycle and responses.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/query-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use serde::{Deserialize, Serialize};

use super::base::Query;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Status of a query cursor.
///
/// Maps to `CursorStatusSchema` in `query-schemas.ts`.
/// Lowercase variant names match the TS enum values exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStatus {
    /// Cursor is valid and can be used for pagination.
    Valid,
    /// Cursor has expired and must be re-created.
    Expired,
    /// Cursor is invalid (e.g., malformed).
    Invalid,
    /// No cursor available.
    None,
}

// ---------------------------------------------------------------------------
// Query subscription messages
// ---------------------------------------------------------------------------

/// Payload for a query subscription request.
///
/// Maps to the `payload` of `QuerySubMessageSchema` in `query-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySubPayload {
    /// Unique identifier for this subscription.
    pub query_id: String,
    /// Name of the map to query.
    pub map_name: String,
    /// The query parameters (filter, sort, pagination).
    pub query: Query,
}

/// Query subscription request message.
///
/// Maps to `QuerySubMessageSchema` in `query-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySubMessage {
    /// Always `"QUERY_SUB"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The subscription payload.
    pub payload: QuerySubPayload,
}

/// Payload for a query unsubscription request.
///
/// Maps to the `payload` of `QueryUnsubMessageSchema` in `query-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryUnsubPayload {
    /// Unique identifier of the subscription to cancel.
    pub query_id: String,
}

/// Query unsubscription request message.
///
/// Maps to `QueryUnsubMessageSchema` in `query-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryUnsubMessage {
    /// Always `"QUERY_UNSUB"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The unsubscription payload.
    pub payload: QueryUnsubPayload,
}

// ---------------------------------------------------------------------------
// Query response messages
// ---------------------------------------------------------------------------

/// A single key-value entry in a query response result set.
///
/// Inline type in `QueryRespPayloadSchema` results array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResultEntry {
    /// The key of the matching record.
    pub key: String,
    /// The record value (dynamic type).
    pub value: rmpv::Value,
}

/// Payload for a query response message.
///
/// Maps to `QueryRespPayloadSchema` in `query-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRespPayload {
    /// Identifier of the subscription this response is for.
    pub query_id: String,
    /// The matching records.
    pub results: Vec<QueryResultEntry>,
    /// Optional cursor for fetching the next page.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_cursor: Option<String>,
    /// Optional flag indicating whether more results are available.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub has_more: Option<bool>,
    /// Optional status of the cursor.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor_status: Option<CursorStatus>,
}

/// Query response message containing matching records.
///
/// Maps to `QueryRespMessageSchema` in `query-schemas.ts`.
/// Uses payload wrapper pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRespMessage {
    /// Always `"QUERY_RESP"`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The query response payload.
    pub payload: QueryRespPayload,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::messages::base::{PredicateNode, PredicateOp, SortDirection};

    /// Helper: round-trip a value through named `MsgPack` serialization.
    fn roundtrip_named<T>(val: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let bytes = rmp_serde::to_vec_named(val).expect("serialize");
        rmp_serde::from_slice(&bytes).expect("deserialize")
    }

    #[test]
    fn cursor_status_roundtrip() {
        let variants = vec![
            CursorStatus::Valid,
            CursorStatus::Expired,
            CursorStatus::Invalid,
            CursorStatus::None,
        ];
        for v in &variants {
            assert_eq!(&roundtrip_named(v), v);
        }
    }

    #[test]
    fn cursor_status_serializes_lowercase() {
        let bytes = rmp_serde::to_vec_named(&CursorStatus::Valid).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "valid");

        let bytes = rmp_serde::to_vec_named(&CursorStatus::Expired).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "expired");

        let bytes = rmp_serde::to_vec_named(&CursorStatus::None).unwrap();
        let s: String = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(s, "none");
    }

    #[test]
    fn query_sub_message_roundtrip() {
        let mut where_clause = HashMap::new();
        where_clause.insert("status".to_string(), rmpv::Value::String("active".into()));

        let msg = QuerySubMessage {
            r#type: "QUERY_SUB".to_string(),
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "users".to_string(),
                query: Query {
                    r#where: Some(where_clause),
                    predicate: Some(PredicateNode {
                        op: PredicateOp::Gt,
                        attribute: Some("age".to_string()),
                        value: Some(rmpv::Value::Integer(18.into())),
                        children: None,
                    }),
                    sort: Some({
                        let mut s = HashMap::new();
                        s.insert("name".to_string(), SortDirection::Asc);
                        s
                    }),
                    limit: Some(100),
                    cursor: None,
                },
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn query_unsub_message_roundtrip() {
        let msg = QueryUnsubMessage {
            r#type: "QUERY_UNSUB".to_string(),
            payload: QueryUnsubPayload {
                query_id: "q-1".to_string(),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn query_resp_message_full_roundtrip() {
        let msg = QueryRespMessage {
            r#type: "QUERY_RESP".to_string(),
            payload: QueryRespPayload {
                query_id: "q-1".to_string(),
                results: vec![
                    QueryResultEntry {
                        key: "user-1".to_string(),
                        value: rmpv::Value::Map(vec![
                            (
                                rmpv::Value::String("name".into()),
                                rmpv::Value::String("Alice".into()),
                            ),
                        ]),
                    },
                    QueryResultEntry {
                        key: "user-2".to_string(),
                        value: rmpv::Value::Map(vec![
                            (
                                rmpv::Value::String("name".into()),
                                rmpv::Value::String("Bob".into()),
                            ),
                        ]),
                    },
                ],
                next_cursor: Some("cursor-abc".to_string()),
                has_more: Some(true),
                cursor_status: Some(CursorStatus::Valid),
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn query_resp_message_minimal_roundtrip() {
        let msg = QueryRespMessage {
            r#type: "QUERY_RESP".to_string(),
            payload: QueryRespPayload {
                query_id: "q-2".to_string(),
                results: vec![],
                next_cursor: None,
                has_more: None,
                cursor_status: None,
            },
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    // ---- AC-4: Optional field omission (byte inspection) ----

    #[test]
    fn query_resp_optional_fields_omitted_when_none() {
        // AC-4: When nextCursor, hasMore, cursorStatus are None, keys should not appear
        let msg = QueryRespMessage {
            r#type: "QUERY_RESP".to_string(),
            payload: QueryRespPayload {
                query_id: "q-1".to_string(),
                results: vec![],
                next_cursor: None,
                has_more: None,
                cursor_status: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");

        let payload = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");

        let has_next_cursor = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("nextCursor"));
        assert!(
            !has_next_cursor,
            "nextCursor should be omitted when None"
        );

        let has_has_more = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("hasMore"));
        assert!(!has_has_more, "hasMore should be omitted when None");

        let has_cursor_status = payload_map
            .iter()
            .any(|(k, _)| k.as_str() == Some("cursorStatus"));
        assert!(
            !has_cursor_status,
            "cursorStatus should be omitted when None"
        );
    }

    // ---- camelCase verification ----

    #[test]
    fn query_sub_camel_case_field_names() {
        let msg = QuerySubMessage {
            r#type: "QUERY_SUB".to_string(),
            payload: QuerySubPayload {
                query_id: "q-1".to_string(),
                map_name: "test".to_string(),
                query: Query {
                    r#where: None,
                    predicate: None,
                    sort: None,
                    limit: None,
                    cursor: None,
                },
            },
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        let map = val.as_map().expect("should be a map");

        let payload = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("payload"))
            .map(|(_, v)| v)
            .expect("should have payload");
        let payload_map = payload.as_map().expect("payload should be a map");

        let keys: Vec<&str> = payload_map
            .iter()
            .filter_map(|(k, _)| k.as_str())
            .collect();
        assert!(keys.contains(&"queryId"), "expected camelCase 'queryId'");
        assert!(keys.contains(&"mapName"), "expected camelCase 'mapName'");
    }
}
