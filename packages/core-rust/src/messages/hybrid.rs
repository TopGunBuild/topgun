//! Wire types for hybrid search request/response/subscription messages.
//!
//! Hybrid search combines exact-match, BM25 full-text, and ANN semantic search
//! via RRF (Reciprocal Rank Fusion) into a single fused result list.
//! All types use named `MsgPack` serialization with camelCase field names.

use serde::{Deserialize, Serialize};

use crate::messages::base::{ChangeEventType, PredicateNode};

// ---------------------------------------------------------------------------
// serde_bytes_opt: Option<Vec<u8>> ↔ MsgPack bin / nil
// Duplicated from vector.rs because extracting to messages::util would exceed
// the 5-file Rust limit for this spec.
// TODO: consolidate serde_bytes_opt into messages::util
// ---------------------------------------------------------------------------
mod serde_bytes_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_bytes::ByteBuf;

    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(|b| ByteBuf::from(b.clone())).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<ByteBuf>::deserialize(d).map(|o| o.map(ByteBuf::into_vec))
    }
}

// ---------------------------------------------------------------------------
// SearchMethod enum (wire-only mirror; server-rust has the same enum with
// serde derives pre-positioned by SPEC-206)
// ---------------------------------------------------------------------------

/// Which search methods to invoke in a hybrid search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMethod {
    /// Exact key/value match search.
    Exact,
    /// BM25 full-text search via tantivy.
    FullText,
    /// ANN semantic search via HNSW vector index.
    Semantic,
}

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// One-shot hybrid search request payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchPayload {
    /// Request correlation id (echoed in response).
    pub request_id: String,
    /// Map to search.
    pub map_name: String,
    /// Full-text query string (used by `FullText` and optionally `Semantic` for auto-embed).
    pub query_text: String,
    /// Which search methods to invoke.
    pub methods: Vec<SearchMethod>,
    /// Number of top results to return.
    pub k: u32,
    /// Pre-computed query vector as little-endian f32 bytes. If None and Semantic
    /// is requested, the server auto-embeds from `query_text`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[serde(with = "serde_bytes_opt")]
    pub query_vector: Option<Vec<u8>>,
    /// Post-filter predicate applied to candidate records.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub predicate: Option<PredicateNode>,
    /// If true, populate result values from the record store. Default: true.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub include_value: Option<bool>,
    /// Minimum fused RRF score threshold. Results below this are dropped.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_score: Option<f64>,
}

/// A single entry in a hybrid search response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchResultEntry {
    /// Record key.
    pub key: String,
    /// Fused RRF score (higher is better).
    pub score: f64,
    /// Per-method original scores for transparency.
    pub method_scores: std::collections::HashMap<SearchMethod, f64>,
    /// Full record value (present iff `include_value` != Some(false)).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<rmpv::Value>,
}

/// Response payload for hybrid search.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchRespPayload {
    /// Correlates with `HybridSearchPayload.request_id`.
    pub request_id: String,
    /// Top-k results sorted by descending fused score.
    pub results: Vec<HybridSearchResultEntry>,
    /// Wall-clock search time in milliseconds.
    pub search_time_ms: u64,
    /// Error message if the search failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Payload to subscribe to live hybrid search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchSubPayload {
    /// Unique subscription identifier.
    pub subscription_id: String,
    /// Map to search.
    pub map_name: String,
    /// Full-text query string.
    pub query_text: String,
    /// Which search methods to invoke.
    pub methods: Vec<SearchMethod>,
    /// Number of top results to maintain.
    pub k: u32,
    /// Pre-computed query vector as little-endian f32 bytes.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[serde(with = "serde_bytes_opt")]
    pub query_vector: Option<Vec<u8>>,
    /// Post-filter predicate.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub predicate: Option<PredicateNode>,
    /// If true, populate result values. Default: true.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub include_value: Option<bool>,
    /// Minimum fused RRF score threshold.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_score: Option<f64>,
}

/// Payload for a live hybrid search update (ENTER/UPDATE/LEAVE delta).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchUpdatePayload {
    /// Subscription this update belongs to.
    pub subscription_id: String,
    /// Key of the affected record.
    pub key: String,
    /// Current fused RRF score.
    pub score: f64,
    /// Per-method scores.
    pub method_scores: std::collections::HashMap<SearchMethod, f64>,
    /// Record value (if `include_value` is true).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<rmpv::Value>,
    /// Whether the record entered, updated within, or left the result set.
    pub change_type: ChangeEventType,
}

/// Payload to unsubscribe from a live hybrid search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchUnsubPayload {
    /// Subscription to cancel.
    pub subscription_id: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_named<T: serde::Serialize + for<'a> serde::Deserialize<'a>>(v: &T) -> T {
        let bytes = rmp_serde::to_vec_named(v).unwrap();
        rmp_serde::from_slice(&bytes).unwrap()
    }

    fn make_query_vector_2d() -> Vec<u8> {
        let f1: f32 = 1.0;
        let f2: f32 = 2.0;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&f1.to_le_bytes());
        bytes.extend_from_slice(&f2.to_le_bytes());
        bytes
    }

    fn extract_keys(bytes: &[u8]) -> Vec<String> {
        let val: rmpv::Value = rmp_serde::from_slice(bytes).unwrap();
        match val {
            rmpv::Value::Map(m) => m
                .iter()
                .filter_map(|(k, _)| {
                    if let rmpv::Value::String(s) = k {
                        s.as_str().map(String::from)
                    } else {
                        None
                    }
                })
                .collect(),
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn hybrid_search_payload_roundtrip() {
        let payload = HybridSearchPayload {
            request_id: "req-1".to_string(),
            map_name: "my_map".to_string(),
            query_text: "hello world".to_string(),
            methods: vec![SearchMethod::Exact, SearchMethod::FullText, SearchMethod::Semantic],
            k: 10,
            query_vector: Some(make_query_vector_2d()),
            predicate: None,
            include_value: Some(true),
            min_score: Some(0.5),
        };
        assert_eq!(roundtrip_named(&payload), payload);
    }

    #[test]
    fn hybrid_search_payload_camel_case() {
        let payload = HybridSearchPayload {
            request_id: "req-cc".to_string(),
            map_name: "m".to_string(),
            query_text: "test".to_string(),
            methods: vec![SearchMethod::FullText],
            k: 5,
            query_vector: Some(make_query_vector_2d()),
            predicate: None,
            include_value: Some(false),
            min_score: Some(0.1),
        };
        let bytes = rmp_serde::to_vec_named(&payload).unwrap();
        let keys = extract_keys(&bytes);
        assert!(keys.contains(&"requestId".to_string()), "missing requestId, keys={keys:?}");
        assert!(keys.contains(&"mapName".to_string()), "missing mapName, keys={keys:?}");
        assert!(keys.contains(&"queryText".to_string()), "missing queryText, keys={keys:?}");
        assert!(keys.contains(&"methods".to_string()), "missing methods, keys={keys:?}");
        assert!(keys.contains(&"queryVector".to_string()), "missing queryVector, keys={keys:?}");
        assert!(keys.contains(&"includeValue".to_string()), "missing includeValue, keys={keys:?}");
        assert!(keys.contains(&"minScore".to_string()), "missing minScore, keys={keys:?}");
    }

    #[test]
    fn hybrid_search_payload_minimal() {
        let payload = HybridSearchPayload {
            request_id: "req-min".to_string(),
            map_name: "m".to_string(),
            query_text: "q".to_string(),
            methods: vec![SearchMethod::Exact],
            k: 1,
            query_vector: None,
            predicate: None,
            include_value: None,
            min_score: None,
        };
        let rt = roundtrip_named(&payload);
        assert_eq!(rt, payload);
        assert!(rt.query_vector.is_none());
        assert!(rt.predicate.is_none());
        assert!(rt.include_value.is_none());
        assert!(rt.min_score.is_none());
        // Verify optional fields are omitted from wire
        let bytes = rmp_serde::to_vec_named(&payload).unwrap();
        let keys = extract_keys(&bytes);
        assert!(!keys.contains(&"queryVector".to_string()), "queryVector should be omitted");
        assert!(!keys.contains(&"includeValue".to_string()), "includeValue should be omitted");
        assert!(!keys.contains(&"minScore".to_string()), "minScore should be omitted");
    }

    #[test]
    fn hybrid_search_resp_roundtrip() {
        let entry = HybridSearchResultEntry {
            key: "k1".to_string(),
            score: 0.9,
            method_scores: {
                let mut m = std::collections::HashMap::new();
                m.insert(SearchMethod::Exact, 0.8);
                m.insert(SearchMethod::FullText, 0.7);
                m
            },
            value: None,
        };
        let payload = HybridSearchRespPayload {
            request_id: "req-1".to_string(),
            results: vec![entry],
            search_time_ms: 42,
            error: None,
        };
        assert_eq!(roundtrip_named(&payload), payload);
    }

    #[test]
    fn hybrid_search_resp_error() {
        let payload = HybridSearchRespPayload {
            request_id: "req-err".to_string(),
            results: vec![],
            search_time_ms: 0,
            error: Some("index not found".to_string()),
        };
        let rt = roundtrip_named(&payload);
        assert_eq!(rt, payload);
        assert!(rt.results.is_empty());
        assert_eq!(rt.error, Some("index not found".to_string()));
    }

    #[test]
    fn hybrid_search_sub_payload_roundtrip() {
        let payload = HybridSearchSubPayload {
            subscription_id: "sub-1".to_string(),
            map_name: "my_map".to_string(),
            query_text: "hello".to_string(),
            methods: vec![SearchMethod::FullText, SearchMethod::Semantic],
            k: 5,
            query_vector: Some(make_query_vector_2d()),
            predicate: None,
            include_value: Some(true),
            min_score: None,
        };
        assert_eq!(roundtrip_named(&payload), payload);
    }

    #[test]
    fn hybrid_search_update_payload_roundtrip() {
        let payload = HybridSearchUpdatePayload {
            subscription_id: "sub-1".to_string(),
            key: "k2".to_string(),
            score: 0.75,
            method_scores: {
                let mut m = std::collections::HashMap::new();
                m.insert(SearchMethod::Semantic, 0.75);
                m
            },
            value: None,
            change_type: ChangeEventType::ENTER,
        };
        assert_eq!(roundtrip_named(&payload), payload);
    }

    #[test]
    fn hybrid_search_unsub_payload_roundtrip() {
        let payload = HybridSearchUnsubPayload {
            subscription_id: "sub-1".to_string(),
        };
        assert_eq!(roundtrip_named(&payload), payload);
    }

    #[test]
    fn search_method_serde() {
        // SearchMethod::FullText must serialise as "fullText" (camelCase)
        let bytes = rmp_serde::to_vec_named(&SearchMethod::FullText).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        match &val {
            rmpv::Value::String(s) => {
                assert_eq!(s.as_str(), Some("fullText"), "expected fullText, got {val:?}");
            }
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn method_scores_hashmap_keys() {
        // HashMap<SearchMethod, f64> must produce string keys "exact" and "fullText" on wire
        let mut map = std::collections::HashMap::new();
        map.insert(SearchMethod::Exact, 0.5_f64);
        map.insert(SearchMethod::FullText, 0.8_f64);

        let bytes = rmp_serde::to_vec_named(&map).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let pairs = match val {
            rmpv::Value::Map(m) => m,
            other => panic!("expected Map, got {other:?}"),
        };
        let string_keys: Vec<String> = pairs
            .iter()
            .filter_map(|(k, _)| {
                if let rmpv::Value::String(s) = k {
                    s.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            string_keys.contains(&"exact".to_string()),
            "missing key 'exact', keys={string_keys:?}"
        );
        assert!(
            string_keys.contains(&"fullText".to_string()),
            "missing key 'fullText', keys={string_keys:?}"
        );

        // Verify roundtrip preserves values
        let rt: std::collections::HashMap<SearchMethod, f64> =
            rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(rt.get(&SearchMethod::Exact), Some(&0.5));
        assert_eq!(rt.get(&SearchMethod::FullText), Some(&0.8));
    }
}
