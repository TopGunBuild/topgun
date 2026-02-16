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
