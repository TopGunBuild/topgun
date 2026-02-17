//! Search domain payload structs for full-text search requests, responses,
//! and live subscriptions.
//!
//! These types correspond to the TypeScript Zod schemas in
//! `packages/core/src/schemas/search-schemas.ts`. All structs use
//! `#[serde(rename_all = "camelCase")]` to produce wire-compatible
//! `MsgPack` output via `rmp_serde::to_vec_named()`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::base::ChangeEventType;

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// Options for search requests and subscriptions.
///
/// Maps to `SearchOptionsSchema` in `search-schemas.ts`.
/// All fields are optional; derives `Default` for ergonomic construction.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    /// Maximum number of results to return.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,

    /// Minimum relevance score threshold.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_score: Option<f64>,

    /// Per-field boost weights for search ranking.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub boost: Option<HashMap<String, f64>>,
}

/// A single entry in a search response result set.
///
/// Maps to the inline result type in `SearchRespPayloadSchema.results`
/// in `search-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultEntry {
    /// The key of the matched record.
    pub key: String,

    /// The full record value.
    pub value: rmpv::Value,

    /// Relevance score for this result.
    pub score: f64,

    /// Terms from the query that matched this record.
    pub matched_terms: Vec<String>,
}

// ---------------------------------------------------------------------------
// Request / response payloads
// ---------------------------------------------------------------------------

/// Payload for a one-shot search request.
///
/// Maps to `SearchPayloadSchema` in `search-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPayload {
    /// Unique identifier for correlating request with response.
    pub request_id: String,

    /// Name of the map to search.
    pub map_name: String,

    /// Full-text search query string.
    pub query: String,

    /// Optional search configuration.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<SearchOptions>,
}

/// Payload for a search response.
///
/// Maps to `SearchRespPayloadSchema` in `search-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRespPayload {
    /// Matches the `request_id` from the originating `SearchPayload`.
    pub request_id: String,

    /// Matched records with scores and terms.
    pub results: Vec<SearchResultEntry>,

    /// Total number of matching records (may exceed `results.len()` if limited).
    pub total_count: u32,

    /// Error message if the search failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Live search subscription payloads
// ---------------------------------------------------------------------------

/// Payload to subscribe to live search results.
///
/// Maps to `SearchSubPayloadSchema` in `search-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSubPayload {
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

/// Payload for a live search update notification.
///
/// Maps to `SearchUpdatePayloadSchema` in `search-schemas.ts`.
/// Uses `ChangeEventType` from `base.rs` (aliased as `SearchUpdateType` in TS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchUpdatePayload {
    /// Subscription this update belongs to.
    pub subscription_id: String,

    /// Key of the record that changed.
    pub key: String,

    /// Current value of the record.
    pub value: rmpv::Value,

    /// Updated relevance score.
    pub score: f64,

    /// Terms from the query that match the updated record.
    pub matched_terms: Vec<String>,

    /// Whether the record entered, updated within, or left the result set.
    pub change_type: ChangeEventType,
}

/// Payload to unsubscribe from a live search.
///
/// Maps to `SearchUnsubPayloadSchema` in `search-schemas.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchUnsubPayload {
    /// Subscription to cancel.
    pub subscription_id: String,
}
