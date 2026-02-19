//! Messaging domain payload structs for topics, locks, PN counters, heartbeat,
//! entry processors, journal events, and conflict resolvers.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/messaging-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::hlc::{serde_number, Timestamp};

// ---------------------------------------------------------------------------
// Topic Payloads
// ---------------------------------------------------------------------------

/// Payload for `TOPIC_SUB` (payload-wrapped).
///
/// Maps to `TopicSubSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicSubPayload {
    /// Topic name to subscribe to.
    pub topic: String,
}

/// Payload for `TOPIC_UNSUB` (payload-wrapped).
///
/// Maps to `TopicUnsubSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicUnsubPayload {
    /// Topic name to unsubscribe from.
    pub topic: String,
}

/// Payload for `TOPIC_PUB` (payload-wrapped).
///
/// Maps to `TopicPubSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicPubPayload {
    /// Topic name to publish to.
    pub topic: String,

    /// Arbitrary data payload.
    pub data: rmpv::Value,
}

/// Payload for `TOPIC_MESSAGE` (payload-wrapped).
///
/// Maps to `TopicMessageEventSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicMessageEventPayload {
    /// Topic name.
    pub topic: String,

    /// Arbitrary data payload.
    pub data: rmpv::Value,

    /// Optional publisher identifier.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub publisher_id: Option<String>,

    /// Timestamp in milliseconds.
    #[serde(deserialize_with = "serde_number::deserialize_u64")]
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Lock Payloads
// ---------------------------------------------------------------------------

/// Payload for `LOCK_REQUEST` (payload-wrapped).
///
/// Maps to `LockRequestSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRequestPayload {
    /// Unique request identifier.
    pub request_id: String,

    /// Lock name.
    pub name: String,

    /// Optional timeout in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none", default, deserialize_with = "serde_number::deserialize_option_u64")]
    pub ttl: Option<u64>,
}

/// Payload for `LOCK_RELEASE` (payload-wrapped).
///
/// Maps to `LockReleaseSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockReleasePayload {
    /// Optional request identifier.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,

    /// Lock name.
    pub name: String,

    /// Monotonic counter fencing token.
    #[serde(deserialize_with = "serde_number::deserialize_u64")]
    pub fencing_token: u64,
}

// ---------------------------------------------------------------------------
// PN Counter
// ---------------------------------------------------------------------------

/// Per-node positive/negative counter state for PN-Counter CRDT.
///
/// Maps to `PNCounterStateObjectSchema` in `messaging-schemas.ts`.
/// Field names `p` and `n` are single-character and unaffected by camelCase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PNCounterState {
    /// Per-node increment values (may be fractional).
    pub p: HashMap<String, f64>,

    /// Per-node decrement values (may be fractional).
    pub n: HashMap<String, f64>,
}

/// Payload for `COUNTER_REQUEST` (payload-wrapped).
///
/// Maps to `CounterRequestSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterRequestPayload {
    /// Counter name.
    pub name: String,
}

/// Shared payload for `COUNTER_SYNC`, `COUNTER_RESPONSE`, and `COUNTER_UPDATE`
/// (all payload-wrapped).
///
/// Maps to `CounterSyncSchema.payload` / `CounterResponseSchema.payload` /
/// `CounterUpdateSchema.payload` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterStatePayload {
    /// Counter name.
    pub name: String,

    /// PN-Counter state.
    pub state: PNCounterState,
}

// ---------------------------------------------------------------------------
// Heartbeat (Flat)
// ---------------------------------------------------------------------------

/// Flat fields for `PING` message (minus the `type` discriminant).
///
/// Maps to `PingMessageSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingData {
    /// Milliseconds since epoch.
    #[serde(deserialize_with = "serde_number::deserialize_u64")]
    pub timestamp: u64,
}

/// Flat fields for `PONG` message (minus the `type` discriminant).
///
/// Maps to `PongMessageSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PongData {
    /// Client-sent timestamp (echo), milliseconds since epoch.
    #[serde(deserialize_with = "serde_number::deserialize_u64")]
    pub timestamp: u64,

    /// Server time, milliseconds since epoch.
    #[serde(deserialize_with = "serde_number::deserialize_u64")]
    pub server_time: u64,
}

// ---------------------------------------------------------------------------
// Entry Processor (Flat)
// ---------------------------------------------------------------------------

/// Sub-type representing a user-defined entry processor.
///
/// Maps to `EntryProcessorSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryProcessor {
    /// Processor name (1..100 chars).
    pub name: String,

    /// Processor source code (1..10000 chars).
    pub code: String,

    /// Optional arguments to pass to the processor.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub args: Option<rmpv::Value>,
}

/// Flat fields for `ENTRY_PROCESS` message (minus the `type` discriminant).
///
/// Maps to `EntryProcessRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryProcessData {
    /// Unique request identifier.
    pub request_id: String,

    /// Target map name.
    pub map_name: String,

    /// Target key within the map.
    pub key: String,

    /// The entry processor to execute.
    pub processor: EntryProcessor,
}

/// Flat fields for `ENTRY_PROCESS_BATCH` message (minus the `type` discriminant).
///
/// Maps to `EntryProcessBatchRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryProcessBatchData {
    /// Unique request identifier.
    pub request_id: String,

    /// Target map name.
    pub map_name: String,

    /// Target keys within the map.
    pub keys: Vec<String>,

    /// The entry processor to execute on each key.
    pub processor: EntryProcessor,
}

/// Per-key result in an entry process batch response.
///
/// Maps to `EntryProcessKeyResultSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryProcessKeyResult {
    /// Whether processing succeeded for this key.
    pub success: bool,

    /// Return value from the processor, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<rmpv::Value>,

    /// New value written to the key, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub new_value: Option<rmpv::Value>,

    /// Error message if processing failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Flat fields for `ENTRY_PROCESS_RESPONSE` message (minus the `type` discriminant).
///
/// Maps to `EntryProcessResponseSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryProcessResponseData {
    /// Unique request identifier.
    pub request_id: String,

    /// Whether processing succeeded.
    pub success: bool,

    /// Return value from the processor, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<rmpv::Value>,

    /// New value written, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub new_value: Option<rmpv::Value>,

    /// Error message if processing failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Flat fields for `ENTRY_PROCESS_BATCH_RESPONSE` message (minus the `type` discriminant).
///
/// Maps to `EntryProcessBatchResponseSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryProcessBatchResponseData {
    /// Unique request identifier.
    pub request_id: String,

    /// Per-key results keyed by the processed key name.
    pub results: HashMap<String, EntryProcessKeyResult>,
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

/// Classification of a journal event.
///
/// Maps to `JournalEventTypeSchema = z.enum(['PUT', 'UPDATE', 'DELETE'])` in
/// `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum JournalEventType {
    /// A new key-value pair was inserted.
    #[default]
    PUT,
    /// An existing key-value pair was modified.
    UPDATE,
    /// A key-value pair was removed.
    DELETE,
}

/// A single journal entry recording a mutation event.
///
/// Maps to `JournalEventDataSchema` in `messaging-schemas.ts`.
///
/// The `event_type` field is renamed to `"type"` on the wire via
/// `#[serde(rename = "type")]` because the TS schema uses `type` as the
/// field name. This does not conflict with the message-level `type`
/// discriminant, which is managed by the `Message` enum (SPEC-052e).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEventData {
    /// Monotonic sequence identifier.
    pub sequence: String,

    /// The kind of mutation that occurred.
    #[serde(rename = "type")]
    pub event_type: JournalEventType,

    /// Map that was mutated.
    pub map_name: String,

    /// Key that was mutated.
    pub key: String,

    /// New value, if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<rmpv::Value>,

    /// Previous value before mutation, if applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub previous_value: Option<rmpv::Value>,

    /// HLC timestamp of the mutation.
    pub timestamp: Timestamp,

    /// Node that performed the mutation.
    pub node_id: String,

    /// Optional metadata associated with the event.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<HashMap<String, rmpv::Value>>,
}

/// Manual `Default` implementation because `Timestamp` does not derive `Default`.
impl Default for JournalEventData {
    fn default() -> Self {
        Self {
            sequence: String::new(),
            event_type: JournalEventType::PUT,
            map_name: String::new(),
            key: String::new(),
            value: None,
            previous_value: None,
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: String::new(),
            },
            node_id: String::new(),
            metadata: None,
        }
    }
}

/// Flat fields for `JOURNAL_SUBSCRIBE` message (minus the `type` discriminant).
///
/// Maps to `JournalSubscribeRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalSubscribeData {
    /// Unique request identifier.
    pub request_id: String,

    /// Sequence to start reading from.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub from_sequence: Option<String>,

    /// Optional map name filter.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub map_name: Option<String>,

    /// Optional event type filter.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub types: Option<Vec<JournalEventType>>,
}

/// Flat fields for `JOURNAL_UNSUBSCRIBE` message (minus the `type` discriminant).
///
/// Maps to `JournalUnsubscribeRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalUnsubscribeData {
    /// Subscription to cancel.
    pub subscription_id: String,
}

/// Flat fields for `JOURNAL_EVENT` wire message (minus the `type` discriminant).
///
/// Wraps a single `JournalEventData`. Named `JournalEventMessageData` to avoid
/// collision with the `JournalEventData` sub-type.
///
/// Maps to `JournalEventMessageSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEventMessageData {
    /// The journal event.
    pub event: JournalEventData,
}

/// Flat fields for `JOURNAL_READ` message (minus the `type` discriminant).
///
/// Maps to `JournalReadRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalReadData {
    /// Unique request identifier.
    pub request_id: String,

    /// Sequence to start reading from.
    pub from_sequence: String,

    /// Optional page size limit.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,

    /// Optional map name filter.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub map_name: Option<String>,
}

/// Flat fields for `JOURNAL_READ_RESPONSE` message (minus the `type` discriminant).
///
/// Maps to `JournalReadResponseSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalReadResponseData {
    /// Unique request identifier.
    pub request_id: String,

    /// Journal events in the requested range.
    pub events: Vec<JournalEventData>,

    /// Whether more events are available beyond this page.
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// Conflict Resolver
// ---------------------------------------------------------------------------

/// Sub-type representing a user-defined conflict resolver.
///
/// Maps to `ConflictResolverSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictResolver {
    /// Resolver name (1..100 chars).
    pub name: String,

    /// Resolver source code (max 50000 chars).
    pub code: String,

    /// Optional priority (0..100).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub priority: Option<u32>,

    /// Optional key pattern for selective resolution.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub key_pattern: Option<String>,
}

/// Flat fields for `REGISTER_RESOLVER` message (minus the `type` discriminant).
///
/// Maps to `RegisterResolverRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResolverData {
    /// Unique request identifier.
    pub request_id: String,

    /// Target map name.
    pub map_name: String,

    /// The conflict resolver to register.
    pub resolver: ConflictResolver,
}

/// Flat fields for `REGISTER_RESOLVER_RESPONSE` message (minus the `type` discriminant).
///
/// Maps to `RegisterResolverResponseSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResolverResponseData {
    /// Unique request identifier.
    pub request_id: String,

    /// Whether registration succeeded.
    pub success: bool,

    /// Error message if registration failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Flat fields for `UNREGISTER_RESOLVER` message (minus the `type` discriminant).
///
/// Maps to `UnregisterResolverRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnregisterResolverData {
    /// Unique request identifier.
    pub request_id: String,

    /// Target map name.
    pub map_name: String,

    /// Name of the resolver to remove.
    pub resolver_name: String,
}

/// Flat fields for `UNREGISTER_RESOLVER_RESPONSE` message (minus the `type` discriminant).
///
/// Maps to `UnregisterResolverResponseSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnregisterResolverResponseData {
    /// Unique request identifier.
    pub request_id: String,

    /// Whether unregistration succeeded.
    pub success: bool,

    /// Error message if unregistration failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Flat fields for `MERGE_REJECTED` message (minus the `type` discriminant).
///
/// Maps to `MergeRejectedMessageSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRejectedData {
    /// Map that was targeted.
    pub map_name: String,

    /// Key that was targeted.
    pub key: String,

    /// The value that was rejected.
    pub attempted_value: rmpv::Value,

    /// Reason for rejection.
    pub reason: String,

    /// HLC timestamp of the rejected operation.
    pub timestamp: Timestamp,
}

/// Information about a registered conflict resolver.
///
/// Maps to the inline object type in `ListResolversResponseSchema.resolvers`
/// in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverInfo {
    /// Map name this resolver is registered on.
    pub map_name: String,

    /// Resolver name.
    pub name: String,

    /// Optional priority (0..100).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub priority: Option<u32>,

    /// Optional key pattern for selective resolution.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub key_pattern: Option<String>,
}

/// Flat fields for `LIST_RESOLVERS` message (minus the `type` discriminant).
///
/// Maps to `ListResolversRequestSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResolversData {
    /// Unique request identifier.
    pub request_id: String,

    /// Optional map name filter.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub map_name: Option<String>,
}

/// Flat fields for `LIST_RESOLVERS_RESPONSE` message (minus the `type` discriminant).
///
/// Maps to `ListResolversResponseSchema` in `messaging-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResolversResponseData {
    /// Unique request identifier.
    pub request_id: String,

    /// Registered resolvers.
    pub resolvers: Vec<ResolverInfo>,
}
