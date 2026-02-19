//! HTTP sync domain types for the POST `/sync` transport.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/http-sync-schemas.ts`. They are standalone
//! structs used as HTTP request/response bodies, NOT `Message` enum variants.
//! All structs use `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use serde::{Deserialize, Serialize};

use crate::hlc::{LWWRecord, Timestamp};

use super::base::ClientOp;
use super::sync::OpResult;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Event type for delta records in HTTP sync responses.
///
/// Classifies whether a delta record represents a new/updated entry (`PUT`)
/// or a removed entry (`REMOVE`). This is distinct from `ServerEventType`
/// which tracks CRDT-level push events including `ORMap` operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum DeltaRecordEventType {
    PUT,
    REMOVE,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Individual sync map entry specifying which map the client wants deltas for.
///
/// Maps to `SyncMapEntrySchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncMapEntry {
    /// Name of the map to synchronize.
    pub map_name: String,
    /// Timestamp of the client's last successful sync for this map.
    pub last_sync_timestamp: Timestamp,
}

/// One-shot query request over HTTP.
///
/// Maps to `HttpQueryRequestSchema` in `http-sync-schemas.ts`.
/// The `filter` field uses `rmpv::Value` for the TS `z.any()` type.
/// `Default` produces `filter: Value::Nil` with empty strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpQueryRequest {
    /// Unique identifier for this query request.
    pub query_id: String,
    /// Name of the map to query.
    pub map_name: String,
    /// Query filter (arbitrary structure, maps to `z.any()` in TS).
    pub filter: rmpv::Value,
    /// Maximum number of results to return.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,
    /// Number of results to skip for pagination.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub offset: Option<u32>,
}

impl Default for HttpQueryRequest {
    fn default() -> Self {
        Self {
            query_id: String::new(),
            map_name: String::new(),
            filter: rmpv::Value::Nil,
            limit: None,
            offset: None,
        }
    }
}

/// One-shot search request over HTTP.
///
/// Maps to `HttpSearchRequestSchema` in `http-sync-schemas.ts`.
/// The `options` field uses `Option<rmpv::Value>` for the TS `z.any().optional()` type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpSearchRequest {
    /// Unique identifier for this search request.
    pub search_id: String,
    /// Name of the map to search.
    pub map_name: String,
    /// Search query string.
    pub query: String,
    /// Optional search options (arbitrary structure, maps to `z.any().optional()` in TS).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<rmpv::Value>,
}

/// HTTP sync request body sent by the client as POST `/sync`.
///
/// Maps to `HttpSyncRequestSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpSyncRequest {
    /// Client's unique identifier.
    pub client_id: String,
    /// Client's current hybrid logical clock timestamp.
    pub client_hlc: Timestamp,
    /// Optional batch of client operations to apply.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub operations: Option<Vec<ClientOp>>,
    /// Optional list of maps to request deltas for.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sync_maps: Option<Vec<SyncMapEntry>>,
    /// Optional one-shot queries to execute.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub queries: Option<Vec<HttpQueryRequest>>,
    /// Optional one-shot searches to execute.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub searches: Option<Vec<HttpSearchRequest>>,
}

impl Default for HttpSyncRequest {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_hlc: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
            operations: None,
            sync_maps: None,
            queries: None,
            searches: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Delta record for a single key within a map.
///
/// Maps to `DeltaRecordSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaRecord {
    /// Key of the record that changed.
    pub key: String,
    /// The LWW record containing the value and timestamp.
    pub record: LWWRecord<rmpv::Value>,
    /// Whether this is a put or remove operation.
    pub event_type: DeltaRecordEventType,
}

/// Delta records for a specific map, containing all new/changed records since
/// the client's last sync timestamp.
///
/// Maps to `MapDeltaSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapDelta {
    /// Name of the map these deltas belong to.
    pub map_name: String,
    /// Delta records for this map.
    pub records: Vec<DeltaRecord>,
    /// Server's sync timestamp for this batch of deltas.
    pub server_sync_timestamp: Timestamp,
}

/// Query result for a one-shot HTTP query.
///
/// Maps to `HttpQueryResultSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpQueryResult {
    /// Identifier matching the original `HttpQueryRequest.query_id`.
    pub query_id: String,
    /// Query result entries (arbitrary values, maps to `z.array(z.any())` in TS).
    pub results: Vec<rmpv::Value>,
    /// Whether more results are available for pagination.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub has_more: Option<bool>,
    /// Cursor for fetching the next page of results.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_cursor: Option<String>,
}

/// Search result for a one-shot HTTP search.
///
/// Maps to `HttpSearchResultSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpSearchResult {
    /// Identifier matching the original `HttpSearchRequest.search_id`.
    pub search_id: String,
    /// Search result entries (arbitrary values, maps to `z.array(z.any())` in TS).
    pub results: Vec<rmpv::Value>,
    /// Total count of matching results (may exceed `results.len()` for paginated responses).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_count: Option<u32>,
}

/// Error entry for individual operation failures in an HTTP sync response.
///
/// Maps to `HttpSyncErrorSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpSyncError {
    /// Numeric error code.
    pub code: u32,
    /// Human-readable error message.
    pub message: String,
    /// Optional context for the error (e.g., which operation failed).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context: Option<String>,
}

/// Acknowledgment of received operations in an HTTP sync response.
///
/// Corresponds to the inline `z.object({ lastId, results? })` inside
/// `HttpSyncResponseSchema.ack`. Inline TS objects become named structs in Rust.
///
/// `Eq` is correct: `OpResult` contains `WriteConcern` (enum, `Eq`) and
/// `Option<String>` (`Eq`), so the transitive `Eq` derivation is sound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpSyncAck {
    /// Identifier of the last acknowledged operation.
    pub last_id: String,
    /// Optional per-operation results.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub results: Option<Vec<OpResult>>,
}

/// HTTP sync response returned by the server for POST `/sync`.
///
/// Maps to `HttpSyncResponseSchema` in `http-sync-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpSyncResponse {
    /// Server's current hybrid logical clock timestamp.
    pub server_hlc: Timestamp,
    /// Optional acknowledgment of received operations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ack: Option<HttpSyncAck>,
    /// Optional delta records for requested maps.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub deltas: Option<Vec<MapDelta>>,
    /// Optional results for one-shot queries.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub query_results: Option<Vec<HttpQueryResult>>,
    /// Optional results for one-shot searches.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub search_results: Option<Vec<HttpSearchResult>>,
    /// Optional errors for failed operations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub errors: Option<Vec<HttpSyncError>>,
}

impl Default for HttpSyncResponse {
    fn default() -> Self {
        Self {
            server_hlc: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
            ack: None,
            deltas: None,
            query_results: None,
            search_results: None,
            errors: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::Timestamp;

    fn test_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "node-1".into(),
        }
    }

    // ---- DeltaRecordEventType ----

    #[test]
    fn delta_record_event_type_roundtrip() {
        for evt in [DeltaRecordEventType::PUT, DeltaRecordEventType::REMOVE] {
            let bytes = rmp_serde::to_vec_named(&evt).expect("serialize");
            let decoded: DeltaRecordEventType =
                rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(evt, decoded);
        }
    }

    // ---- SyncMapEntry ----

    #[test]
    fn sync_map_entry_roundtrip() {
        let entry = SyncMapEntry {
            map_name: "users".into(),
            last_sync_timestamp: test_timestamp(),
        };
        let bytes = rmp_serde::to_vec_named(&entry).expect("serialize");
        let decoded: SyncMapEntry = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(entry, decoded);
    }

    // ---- HttpQueryRequest ----

    #[test]
    fn http_query_request_roundtrip_all_fields() {
        let req = HttpQueryRequest {
            query_id: "q-001".into(),
            map_name: "products".into(),
            filter: rmpv::Value::Map(vec![(
                rmpv::Value::String("status".into()),
                rmpv::Value::String("active".into()),
            )]),
            limit: Some(50),
            offset: Some(10),
        };
        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let decoded: HttpQueryRequest = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn http_query_request_roundtrip_no_optional() {
        let req = HttpQueryRequest {
            query_id: "q-002".into(),
            map_name: "users".into(),
            filter: rmpv::Value::Nil,
            limit: None,
            offset: None,
        };
        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let decoded: HttpQueryRequest = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn http_query_request_default() {
        let req = HttpQueryRequest::default();
        assert_eq!(req.filter, rmpv::Value::Nil);
        assert!(req.limit.is_none());
        assert!(req.offset.is_none());
    }

    // ---- HttpSearchRequest ----

    #[test]
    fn http_search_request_roundtrip_with_options() {
        let req = HttpSearchRequest {
            search_id: "s-001".into(),
            map_name: "articles".into(),
            query: "rust async".into(),
            options: Some(rmpv::Value::Map(vec![(
                rmpv::Value::String("limit".into()),
                rmpv::Value::Integer(10.into()),
            )])),
        };
        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let decoded: HttpSearchRequest = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn http_search_request_roundtrip_no_options() {
        let req = HttpSearchRequest {
            search_id: "s-002".into(),
            map_name: "docs".into(),
            query: "tutorial".into(),
            options: None,
        };
        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let decoded: HttpSearchRequest = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(req, decoded);
    }

    // ---- HttpSyncRequest ----

    #[test]
    fn http_sync_request_roundtrip_full() {
        let req = HttpSyncRequest {
            client_id: "client-1".into(),
            client_hlc: test_timestamp(),
            operations: Some(vec![ClientOp {
                id: Some("op-1".into()),
                map_name: "users".into(),
                key: "user-1".into(),
                record: Some(Some(LWWRecord {
                    value: Some(rmpv::Value::String("Alice".into())),
                    timestamp: test_timestamp(),
                    ttl_ms: None,
                })),
                ..Default::default()
            }]),
            sync_maps: Some(vec![SyncMapEntry {
                map_name: "users".into(),
                last_sync_timestamp: test_timestamp(),
            }]),
            queries: None,
            searches: None,
        };
        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let decoded: HttpSyncRequest = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn http_sync_request_roundtrip_minimal() {
        let req = HttpSyncRequest {
            client_id: "client-2".into(),
            client_hlc: test_timestamp(),
            ..Default::default()
        };
        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let decoded: HttpSyncRequest = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(req, decoded);
    }

    // ---- DeltaRecord ----

    #[test]
    fn delta_record_roundtrip_put() {
        let record = DeltaRecord {
            key: "user-1".into(),
            record: LWWRecord {
                value: Some(rmpv::Value::String("Alice".into())),
                timestamp: test_timestamp(),
                ttl_ms: None,
            },
            event_type: DeltaRecordEventType::PUT,
        };
        let bytes = rmp_serde::to_vec_named(&record).expect("serialize");
        let decoded: DeltaRecord = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(record, decoded);
    }

    #[test]
    fn delta_record_roundtrip_remove() {
        let record = DeltaRecord {
            key: "user-2".into(),
            record: LWWRecord {
                value: None,
                timestamp: test_timestamp(),
                ttl_ms: None,
            },
            event_type: DeltaRecordEventType::REMOVE,
        };
        let bytes = rmp_serde::to_vec_named(&record).expect("serialize");
        let decoded: DeltaRecord = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(record, decoded);
    }

    // ---- MapDelta ----

    #[test]
    fn map_delta_roundtrip() {
        let delta = MapDelta {
            map_name: "users".into(),
            records: vec![DeltaRecord {
                key: "user-1".into(),
                record: LWWRecord {
                    value: Some(rmpv::Value::String("Alice".into())),
                    timestamp: test_timestamp(),
                    ttl_ms: None,
                },
                event_type: DeltaRecordEventType::PUT,
            }],
            server_sync_timestamp: test_timestamp(),
        };
        let bytes = rmp_serde::to_vec_named(&delta).expect("serialize");
        let decoded: MapDelta = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(delta, decoded);
    }

    // ---- HttpQueryResult ----

    #[test]
    fn http_query_result_roundtrip_all_fields() {
        let result = HttpQueryResult {
            query_id: "q-001".into(),
            results: vec![
                rmpv::Value::String("result-1".into()),
                rmpv::Value::Integer(42.into()),
            ],
            has_more: Some(true),
            next_cursor: Some("cursor-abc".into()),
        };
        let bytes = rmp_serde::to_vec_named(&result).expect("serialize");
        let decoded: HttpQueryResult = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(result, decoded);
    }

    #[test]
    fn http_query_result_roundtrip_no_optional() {
        let result = HttpQueryResult {
            query_id: "q-002".into(),
            results: vec![],
            has_more: None,
            next_cursor: None,
        };
        let bytes = rmp_serde::to_vec_named(&result).expect("serialize");
        let decoded: HttpQueryResult = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(result, decoded);
    }

    // ---- HttpSearchResult ----

    #[test]
    fn http_search_result_roundtrip_with_count() {
        let result = HttpSearchResult {
            search_id: "s-001".into(),
            results: vec![rmpv::Value::String("doc-1".into())],
            total_count: Some(100),
        };
        let bytes = rmp_serde::to_vec_named(&result).expect("serialize");
        let decoded: HttpSearchResult = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(result, decoded);
    }

    #[test]
    fn http_search_result_roundtrip_no_count() {
        let result = HttpSearchResult {
            search_id: "s-002".into(),
            results: vec![],
            total_count: None,
        };
        let bytes = rmp_serde::to_vec_named(&result).expect("serialize");
        let decoded: HttpSearchResult = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(result, decoded);
    }

    // ---- HttpSyncError ----

    #[test]
    fn http_sync_error_roundtrip_with_context() {
        let err = HttpSyncError {
            code: 400,
            message: "invalid operation".into(),
            context: Some("op-42".into()),
        };
        let bytes = rmp_serde::to_vec_named(&err).expect("serialize");
        let decoded: HttpSyncError = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(err, decoded);
    }

    #[test]
    fn http_sync_error_roundtrip_no_context() {
        let err = HttpSyncError {
            code: 500,
            message: "internal error".into(),
            context: None,
        };
        let bytes = rmp_serde::to_vec_named(&err).expect("serialize");
        let decoded: HttpSyncError = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(err, decoded);
    }

    // ---- HttpSyncAck ----

    #[test]
    fn http_sync_ack_roundtrip_with_results() {
        use super::super::base::WriteConcern;

        let ack = HttpSyncAck {
            last_id: "op-99".into(),
            results: Some(vec![OpResult {
                op_id: "op-99".into(),
                success: true,
                achieved_level: WriteConcern::MEMORY,
                error: None,
            }]),
        };
        let bytes = rmp_serde::to_vec_named(&ack).expect("serialize");
        let decoded: HttpSyncAck = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(ack, decoded);
    }

    #[test]
    fn http_sync_ack_roundtrip_no_results() {
        let ack = HttpSyncAck {
            last_id: "op-50".into(),
            results: None,
        };
        let bytes = rmp_serde::to_vec_named(&ack).expect("serialize");
        let decoded: HttpSyncAck = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(ack, decoded);
    }

    // ---- HttpSyncResponse ----

    #[test]
    fn http_sync_response_roundtrip_full() {
        let resp = HttpSyncResponse {
            server_hlc: test_timestamp(),
            ack: Some(HttpSyncAck {
                last_id: "op-1".into(),
                results: None,
            }),
            deltas: Some(vec![MapDelta {
                map_name: "users".into(),
                records: vec![],
                server_sync_timestamp: test_timestamp(),
            }]),
            query_results: Some(vec![HttpQueryResult {
                query_id: "q-1".into(),
                results: vec![],
                ..Default::default()
            }]),
            search_results: None,
            errors: None,
        };
        let bytes = rmp_serde::to_vec_named(&resp).expect("serialize");
        let decoded: HttpSyncResponse = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(resp, decoded);
    }

    #[test]
    fn http_sync_response_roundtrip_minimal() {
        let resp = HttpSyncResponse {
            server_hlc: test_timestamp(),
            ..Default::default()
        };
        let bytes = rmp_serde::to_vec_named(&resp).expect("serialize");
        let decoded: HttpSyncResponse = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(resp, decoded);
    }

    #[test]
    fn http_sync_response_roundtrip_with_errors() {
        let resp = HttpSyncResponse {
            server_hlc: test_timestamp(),
            errors: Some(vec![HttpSyncError {
                code: 409,
                message: "conflict".into(),
                context: Some("op-5".into()),
            }]),
            ..Default::default()
        };
        let bytes = rmp_serde::to_vec_named(&resp).expect("serialize");
        let decoded: HttpSyncResponse = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(resp, decoded);
    }

    // ---- Optional field omission verification ----

    #[test]
    fn optional_fields_omitted_when_none() {
        // Verify that None fields do not produce keys in the serialized MsgPack
        let req = HttpQueryRequest {
            query_id: "q-test".into(),
            map_name: "m".into(),
            filter: rmpv::Value::Nil,
            limit: None,
            offset: None,
        };

        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");

        let limit_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("limit"))
            .collect();
        assert!(
            limit_keys.is_empty(),
            "limit=None should not produce a 'limit' key"
        );

        let offset_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("offset"))
            .collect();
        assert!(
            offset_keys.is_empty(),
            "offset=None should not produce an 'offset' key"
        );
    }

    #[test]
    fn optional_fields_present_when_some() {
        let req = HttpQueryRequest {
            query_id: "q-test".into(),
            map_name: "m".into(),
            filter: rmpv::Value::Nil,
            limit: Some(10),
            offset: Some(5),
        };

        let bytes = rmp_serde::to_vec_named(&req).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");

        let limit_key = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("limit"))
            .expect("limit=Some should produce a 'limit' key");
        assert_eq!(limit_key.1.as_u64(), Some(10));

        let offset_key = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("offset"))
            .expect("offset=Some should produce an 'offset' key");
        assert_eq!(offset_key.1.as_u64(), Some(5));
    }
}
