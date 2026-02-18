//! Client event domain payload structs for server-to-client push messages.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/client-message-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use serde::{Deserialize, Serialize};

use super::base::ChangeEventType;
use crate::hlc::{LWWRecord, ORMapRecord, Timestamp};

// ---------------------------------------------------------------------------
// Server Event
// ---------------------------------------------------------------------------

/// CRDT operation event type for server-to-client push events.
///
/// DISTINCT from `ChangeEventType` (ENTER/UPDATE/LEAVE), which tracks
/// subscription-level changes. `ServerEventType` tracks CRDT-level operations.
///
/// Maps to the inline `z.enum(...)` in `ServerEventPayloadSchema` in
/// `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum ServerEventType {
    /// A key was written (LWW put).
    #[default]
    PUT,
    /// A key was removed (LWW delete).
    REMOVE,
    /// An entry was added to an OR-Map.
    OR_ADD,
    /// An entry was removed from an OR-Map.
    OR_REMOVE,
}

/// Payload for `SERVER_EVENT` (payload-wrapped).
///
/// Maps to `ServerEventPayloadSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEventPayload {
    /// Map that was affected.
    pub map_name: String,

    /// The kind of CRDT operation.
    pub event_type: ServerEventType,

    /// Key that was affected.
    pub key: String,

    /// LWW record for PUT/REMOVE events, if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub record: Option<LWWRecord<rmpv::Value>>,

    /// OR-Map record for `OR_ADD`/`OR_REMOVE` events, if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub or_record: Option<ORMapRecord<rmpv::Value>>,

    /// OR-Map tag for `OR_ADD`/`OR_REMOVE` events, if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub or_tag: Option<String>,
}

/// Payload for `SERVER_BATCH_EVENT` (payload-wrapped).
///
/// Maps to `ServerBatchEventMessageSchema.payload` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerBatchEventPayload {
    /// Batch of server events.
    pub events: Vec<ServerEventPayload>,
}

// ---------------------------------------------------------------------------
// Query Update
// ---------------------------------------------------------------------------

/// Payload for `QUERY_UPDATE` (payload-wrapped).
///
/// Maps to `QueryUpdatePayloadSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryUpdatePayload {
    /// Query identifier.
    pub query_id: String,

    /// Key that changed.
    pub key: String,

    /// Current value.
    pub value: rmpv::Value,

    /// Type of subscription change (ENTER/UPDATE/LEAVE).
    pub change_type: ChangeEventType,
}

// ---------------------------------------------------------------------------
// GC Prune
// ---------------------------------------------------------------------------

/// Payload for `GC_PRUNE` (payload-wrapped).
///
/// Maps to `GcPrunePayloadSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GcPrunePayload {
    /// HLC timestamp threshold: prune entries older than this.
    pub older_than: Timestamp,
}

// ---------------------------------------------------------------------------
// Auth (Flat)
// ---------------------------------------------------------------------------

/// Flat fields for `AUTH_ACK` message (minus the `type` discriminant).
///
/// Maps to `AuthAckMessageSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthAckData {
    /// Optional protocol version negotiated.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub protocol_version: Option<u32>,
}

/// Flat fields for `AUTH_FAIL` message (minus the `type` discriminant).
///
/// Maps to `AuthFailMessageSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthFailData {
    /// Optional error description.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,

    /// Optional error code.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub code: Option<u32>,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Payload for `ERROR` message (payload-wrapped).
///
/// Maps to `ErrorMessageSchema.payload` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    /// Error code.
    pub code: u32,

    /// Error description.
    pub message: String,

    /// Optional additional details.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub details: Option<rmpv::Value>,
}

// ---------------------------------------------------------------------------
// Lock Events
// ---------------------------------------------------------------------------

/// Payload for `LOCK_GRANTED` (payload-wrapped).
///
/// Maps to `LockGrantedPayloadSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockGrantedPayload {
    /// Request identifier that initiated the lock.
    pub request_id: String,

    /// Lock name.
    pub name: String,

    /// Monotonic fencing token for the granted lock.
    pub fencing_token: u64,
}

/// Payload for `LOCK_RELEASED` (payload-wrapped).
///
/// Maps to `LockReleasedPayloadSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockReleasedPayload {
    /// Request identifier.
    pub request_id: String,

    /// Lock name.
    pub name: String,

    /// Whether the lock was successfully released.
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Sync Reset
// ---------------------------------------------------------------------------

/// Payload for `SYNC_RESET_REQUIRED` (payload-wrapped).
///
/// Maps to `SyncResetRequiredPayloadSchema` in `client-message-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResetRequiredPayload {
    /// Map that requires a full re-sync.
    pub map_name: String,

    /// Human-readable reason for the reset.
    pub reason: String,
}
