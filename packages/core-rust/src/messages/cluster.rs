//! Cluster domain payload structs for partition map distribution, inter-node
//! subscription forwarding, and distributed search coordination.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/cluster-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::base::{ChangeEventType, SortDirection};
use super::search::SearchOptions;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Status of a node within the cluster.
///
/// Maps to the inline `z.enum(...)` in `NodeInfoSchema.status` in
/// `cluster-schemas.ts`. Variant names use `SCREAMING_CASE` to match
/// TS wire values directly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum NodeStatus {
    ACTIVE,
    JOINING,
    LEAVING,
    SUSPECTED,
    FAILED,
}

// ---------------------------------------------------------------------------
// Partition Map types
// ---------------------------------------------------------------------------

/// Network endpoints for a cluster node.
///
/// Maps to the inline `endpoints` object in `NodeInfoSchema` in
/// `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEndpoints {
    /// WebSocket endpoint URL.
    pub websocket: String,

    /// Optional HTTP endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub http: Option<String>,
}

/// Information about a single node in the cluster.
///
/// Maps to `NodeInfoSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// Unique identifier for this node.
    pub node_id: String,

    /// Network endpoints for reaching this node.
    pub endpoints: NodeEndpoints,

    /// Current membership status.
    pub status: NodeStatus,
}

/// Ownership information for a single partition.
///
/// Maps to `PartitionInfoSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionInfo {
    /// Partition identifier (0-based).
    pub partition_id: u32,

    /// Node ID of the partition owner.
    pub owner_node_id: String,

    /// Node IDs holding backup replicas.
    pub backup_node_ids: Vec<String>,
}

/// Full partition map describing cluster topology.
///
/// Maps to `PartitionMapPayloadSchema` in `cluster-schemas.ts`.
/// Distributed to clients so they can route operations directly to
/// the owning node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionMapPayload {
    /// Monotonically increasing version for optimistic staleness detection.
    pub version: u32,

    /// Total number of partitions in the cluster (typically 271).
    pub partition_count: u32,

    /// All known cluster nodes and their endpoints.
    pub nodes: Vec<NodeInfo>,

    /// Assignment of partitions to nodes.
    pub partitions: Vec<PartitionInfo>,

    /// Timestamp (ms since epoch) when this map was generated.
    pub generated_at: i64,
}

/// Payload for requesting the current partition map.
///
/// Maps to the `payload` of `PartitionMapRequestSchema` in `cluster-schemas.ts`.
/// Includes the client's current version for delta comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionMapRequestPayload {
    /// Client's current partition map version, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_version: Option<u32>,
}

// ---------------------------------------------------------------------------
// Distributed Live Subscription types
// ---------------------------------------------------------------------------

/// Type of cluster-level subscription being registered.
///
/// Maps to the inline `z.enum(['SEARCH', 'QUERY'])` in
/// `ClusterSubRegisterPayloadSchema.type` in `cluster-schemas.ts`.
/// Derives `Default` with `SEARCH` as the default variant so that
/// `ClusterSubRegisterPayload` can derive `Default`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum ClusterSubType {
    #[default]
    SEARCH,
    QUERY,
}

/// Payload to register a distributed subscription on a remote node.
///
/// Maps to `ClusterSubRegisterPayloadSchema` in `cluster-schemas.ts`.
/// The `sub_type` field serializes as `"type"` on the wire via
/// `#[serde(rename = "type")]`. This does not conflict with the
/// `Message` enum discriminant because payloads are nested.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSubRegisterPayload {
    /// Unique subscription identifier.
    pub subscription_id: String,

    /// Node ID of the subscription coordinator.
    pub coordinator_node_id: String,

    /// Name of the map being subscribed to.
    pub map_name: String,

    /// Whether this is a search or query subscription.
    #[serde(rename = "type")]
    pub sub_type: ClusterSubType,

    /// Full-text search query (for search subscriptions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub search_query: Option<String>,

    /// Search options (for search subscriptions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub search_options: Option<SearchOptions>,

    /// Query predicate tree (for query subscriptions), kept as dynamic value.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub query_predicate: Option<rmpv::Value>,

    /// Sort directives for query subscriptions, reusing `SortDirection` from base.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub query_sort: Option<HashMap<String, SortDirection>>,
}

/// A single entry in the initial result set of a cluster subscription acknowledgment.
///
/// Maps to the inline result object in `ClusterSubAckPayloadSchema.initialResults`
/// in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSubAckResultEntry {
    /// Key of the matching record.
    pub key: String,

    /// Full record value.
    pub value: rmpv::Value,

    /// Optional relevance score (present for search subscriptions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub score: Option<f64>,

    /// Optional matched search terms (present for search subscriptions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub matched_terms: Option<Vec<String>>,
}

impl Default for ClusterSubAckResultEntry {
    fn default() -> Self {
        Self {
            key: String::new(),
            value: rmpv::Value::Nil,
            score: None,
            matched_terms: None,
        }
    }
}

/// Payload acknowledging a cluster subscription registration.
///
/// Maps to `ClusterSubAckPayloadSchema` in `cluster-schemas.ts`.
/// Includes initial results from the local node's data.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSubAckPayload {
    /// Subscription being acknowledged.
    pub subscription_id: String,

    /// Node ID of the acknowledging node.
    pub node_id: String,

    /// Whether the subscription was successfully registered.
    pub success: bool,

    /// Error message if registration failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,

    /// Initial matching records from this node's local data.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub initial_results: Option<Vec<ClusterSubAckResultEntry>>,

    /// Total number of matching records on this node.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_hits: Option<u64>,
}

/// Payload for a live update from a distributed subscription.
///
/// Maps to `ClusterSubUpdatePayloadSchema` in `cluster-schemas.ts`.
/// Forwarded from a data-owning node to the subscription coordinator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSubUpdatePayload {
    /// Subscription this update belongs to.
    pub subscription_id: String,

    /// Node where the change originated.
    pub source_node_id: String,

    /// Key of the changed record.
    pub key: String,

    /// Current value of the record.
    pub value: rmpv::Value,

    /// Relevance score (for search subscriptions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub score: Option<f64>,

    /// Matched search terms (for search subscriptions).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub matched_terms: Option<Vec<String>>,

    /// Whether the record entered, updated within, or left the result set.
    pub change_type: ChangeEventType,

    /// HLC timestamp of the change.
    pub timestamp: u64,
}

impl Default for ClusterSubUpdatePayload {
    fn default() -> Self {
        Self {
            subscription_id: String::new(),
            source_node_id: String::new(),
            key: String::new(),
            value: rmpv::Value::Nil,
            score: None,
            matched_terms: None,
            change_type: ChangeEventType::ENTER,
            timestamp: 0,
        }
    }
}

/// Payload to unregister a distributed subscription on a remote node.
///
/// Maps to `ClusterSubUnregisterPayloadSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSubUnregisterPayload {
    /// Subscription to cancel.
    pub subscription_id: String,
}

// ---------------------------------------------------------------------------
// Distributed Search types
// ---------------------------------------------------------------------------

/// Extended search options for distributed (cluster-level) search requests.
///
/// Maps to the inline `SearchOptionsSchema.extend(...)` in
/// `ClusterSearchReqPayloadSchema.options` in `cluster-schemas.ts`.
/// Unlike `SearchOptions`, `limit` is required here.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchReqOptions {
    /// Maximum number of results to return (required for distributed search).
    pub limit: u32,

    /// Minimum relevance score threshold.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_score: Option<f64>,

    /// Per-field boost weights for search ranking.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub boost: Option<HashMap<String, f64>>,

    /// Whether to include matched terms in results.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub include_matched_terms: Option<bool>,

    /// Cursor: return results after this score (for pagination).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub after_score: Option<f64>,

    /// Cursor: return results after this key (for pagination tie-breaking).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub after_key: Option<String>,
}

/// Payload for a distributed search request sent to cluster nodes.
///
/// Maps to `ClusterSearchReqPayloadSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchReqPayload {
    /// Unique request identifier for correlating responses.
    pub request_id: String,

    /// Name of the map to search.
    pub map_name: String,

    /// Full-text search query string.
    pub query: String,

    /// Extended search options including pagination cursors.
    pub options: ClusterSearchReqOptions,

    /// Maximum time (ms) to wait for results from each node.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timeout_ms: Option<u64>,
}

/// A single entry in a distributed search response.
///
/// Maps to the inline result object in `ClusterSearchRespPayloadSchema.results`
/// in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchResultEntry {
    /// Key of the matched record.
    pub key: String,

    /// Full record value.
    pub value: rmpv::Value,

    /// Relevance score.
    pub score: f64,

    /// Terms that matched the query (optional in distributed context).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub matched_terms: Option<Vec<String>>,
}

/// Payload for a distributed search response from a cluster node.
///
/// Maps to `ClusterSearchRespPayloadSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchRespPayload {
    /// Matches the `request_id` from the originating request.
    pub request_id: String,

    /// Node that produced these results.
    pub node_id: String,

    /// Matching records from this node.
    pub results: Vec<ClusterSearchResultEntry>,

    /// Total number of matching records on this node.
    pub total_hits: u64,

    /// Time (ms) this node spent executing the search.
    pub execution_time_ms: u64,

    /// Error message if the search failed on this node.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Payload to subscribe to live distributed search results.
///
/// Maps to `ClusterSearchSubscribePayloadSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchSubscribePayload {
    /// Unique subscription identifier.
    pub subscription_id: String,

    /// Name of the map to watch.
    pub map_name: String,

    /// Full-text search query string.
    pub query: String,

    /// Optional search configuration.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<SearchOptions>,
}

/// Payload to unsubscribe from a distributed search subscription.
///
/// Maps to `ClusterSearchUnsubscribePayloadSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchUnsubscribePayload {
    /// Subscription to cancel.
    pub subscription_id: String,
}

/// Payload for a live update from a distributed search subscription.
///
/// Maps to `ClusterSearchUpdatePayloadSchema` in `cluster-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSearchUpdatePayload {
    /// Subscription this update belongs to.
    pub subscription_id: String,

    /// Node where the change originated.
    pub node_id: String,

    /// Key of the changed record.
    pub key: String,

    /// Current value of the record.
    pub value: rmpv::Value,

    /// Relevance score of the changed record.
    pub score: f64,

    /// Terms from the query that matched (optional).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub matched_terms: Option<Vec<String>>,

    /// Whether the record entered, updated within, or left the result set.
    pub change_type: ChangeEventType,
}
