//! Wire types for vector search request/response messages.
//!
//! Clients encode a little-endian f32 byte buffer as `query_vector` (`MsgPack`
//! `bin` format). The server decodes it via `decode_query_vector`, runs HNSW
//! nearest-neighbour search, and returns ranked results.

use serde::{Deserialize, Serialize};

use crate::messages::base::PredicateNode;

// ---------------------------------------------------------------------------
// serde_bytes_opt: Option<Vec<u8>> ↔ MsgPack bin / nil
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
// Structs
// ---------------------------------------------------------------------------

/// Per-request knobs for a vector search, modelled after Hazelcast's `SearchOptions`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchOptions {
    /// If `true`, populate `VectorSearchResult.value` from the record store. Default: `true`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub include_value: Option<bool>,

    /// If `true`, populate `VectorSearchResult.vector` with raw attribute bytes. Default: `false`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub include_vectors: Option<bool>,

    /// Minimum similarity score (higher is better). Entries below this are dropped.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_score: Option<f64>,

    /// Post-filter predicate applied to candidate record values before the top-k cut.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<PredicateNode>,
}

/// Request payload: client asks the server to run ANN search on a map's vector index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchPayload {
    /// Request correlation id (echoed in the response).
    pub id: String,
    /// Name of the map to search.
    pub map_name: String,
    /// Attribute name of the vector index. `None` means "the default index for this map".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub index_name: Option<String>,
    /// Query vector as a little-endian f32 byte buffer (length = 4 * dimension).
    #[serde(with = "serde_bytes")]
    pub query_vector: Vec<u8>,
    /// Number of nearest neighbours to return.
    pub k: u32,
    /// Runtime HNSW search precision. If `None`, server uses `k * 2`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ef_search: Option<u32>,
    /// Optional per-request knobs.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<VectorSearchOptions>,
}

impl VectorSearchPayload {
    /// Decodes `query_vector` (little-endian f32 byte buffer) into a native `Vec<f32>`.
    ///
    /// # Errors
    /// Returns `Err` with a descriptive message if the byte length is not a multiple of 4
    /// or if it does not match `4 * expected_dim` when `expected_dim` is provided.
    pub fn decode_query_vector(&self, expected_dim: Option<u16>) -> Result<Vec<f32>, String> {
        if !self.query_vector.len().is_multiple_of(4) {
            return Err(format!(
                "query_vector length {} is not a multiple of 4",
                self.query_vector.len()
            ));
        }
        if let Some(dim) = expected_dim {
            let expected_bytes = (dim as usize) * 4;
            if self.query_vector.len() != expected_bytes {
                return Err(format!(
                    "query_vector length {} does not match expected dimension {} ({} bytes)",
                    self.query_vector.len(),
                    dim,
                    expected_bytes
                ));
            }
        }
        let mut out = Vec::with_capacity(self.query_vector.len() / 4);
        for chunk in self.query_vector.chunks_exact(4) {
            let arr = [chunk[0], chunk[1], chunk[2], chunk[3]];
            out.push(f32::from_le_bytes(arr));
        }
        Ok(out)
    }
}

/// A single entry in a vector search response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchResult {
    /// Record key.
    pub key: String,
    /// Similarity score (higher is better). See R3 for metric → score mapping.
    pub score: f64,
    /// Full record value (present iff `options.include_value != Some(false)`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<rmpv::Value>,
    /// Raw attribute vector bytes (present iff `options.include_vectors == Some(true)`).
    /// Contains little-endian f32 bytes; length == `dimension * 4`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[serde(with = "serde_bytes_opt")]
    pub vector: Option<Vec<u8>>,
}

/// Response payload: ranked nearest-neighbour results.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorSearchRespPayload {
    /// Correlates with `VectorSearchPayload.id`.
    pub id: String,
    /// Top-k results sorted by descending `score`.
    pub results: Vec<VectorSearchResult>,
    /// Total number of candidates considered before top-k / filtering.
    pub total_candidates: u32,
    /// Wall-clock time spent inside `handle_vector_search` in milliseconds.
    pub search_time_ms: u64,
    /// Error message if the search failed. On success, this is `None` and `results` is populated.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
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

    #[test]
    fn vector_search_payload_roundtrip() {
        let msg = VectorSearchPayload {
            id: "req-1".to_string(),
            map_name: "my_map".to_string(),
            index_name: Some("embedding".to_string()),
            query_vector: make_query_vector_2d(),
            k: 5,
            ef_search: Some(10),
            options: Some(VectorSearchOptions {
                include_value: Some(true),
                include_vectors: Some(false),
                min_score: Some(0.8),
                filter: None,
            }),
        };
        assert_eq!(roundtrip_named(&msg), msg);
    }

    #[test]
    fn vector_search_payload_camel_case() {
        let msg = VectorSearchPayload {
            id: "req-camel".to_string(),
            map_name: "map".to_string(),
            index_name: Some("idx".to_string()),
            query_vector: make_query_vector_2d(),
            k: 3,
            ef_search: Some(6),
            options: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = match val {
            rmpv::Value::Map(m) => m,
            other => panic!("expected Map, got {other:?}"),
        };
        let keys: Vec<String> = map
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
            keys.contains(&"mapName".to_string()),
            "missing mapName, keys={keys:?}"
        );
        assert!(
            keys.contains(&"indexName".to_string()),
            "missing indexName, keys={keys:?}"
        );
        assert!(
            keys.contains(&"queryVector".to_string()),
            "missing queryVector, keys={keys:?}"
        );
        assert!(
            keys.contains(&"efSearch".to_string()),
            "missing efSearch, keys={keys:?}"
        );
    }

    #[test]
    fn vector_search_payload_minimal_roundtrip() {
        let msg = VectorSearchPayload {
            id: "min-req".to_string(),
            map_name: "m".to_string(),
            index_name: None,
            query_vector: make_query_vector_2d(),
            k: 1,
            ef_search: None,
            options: None,
        };
        let rt = roundtrip_named(&msg);
        assert_eq!(rt, msg);
        assert!(rt.index_name.is_none());
        assert!(rt.ef_search.is_none());
        assert!(rt.options.is_none());
    }

    #[test]
    fn vector_search_options_default() {
        let opts = VectorSearchOptions::default();
        assert!(opts.include_value.is_none());
        assert!(opts.include_vectors.is_none());
        assert!(opts.min_score.is_none());
        assert!(opts.filter.is_none());
    }

    #[test]
    fn vector_search_options_optional_fields_omitted_when_none() {
        let opts = VectorSearchOptions::default();
        let bytes = rmp_serde::to_vec_named(&opts).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = match val {
            rmpv::Value::Map(m) => m,
            other => panic!("expected Map, got {other:?}"),
        };
        let keys: Vec<String> = map
            .iter()
            .filter_map(|(k, _)| {
                if let rmpv::Value::String(s) = k {
                    s.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect();
        // All optional fields should be absent when None
        assert!(
            !keys.contains(&"minScore".to_string()),
            "minScore should be omitted"
        );
        assert!(
            !keys.contains(&"includeValue".to_string()),
            "includeValue should be omitted"
        );
        assert!(
            !keys.contains(&"includeVectors".to_string()),
            "includeVectors should be omitted"
        );
        assert!(
            !keys.contains(&"filter".to_string()),
            "filter should be omitted"
        );
    }

    #[test]
    fn vector_search_result_with_vector_bytes_roundtrip() {
        let result = VectorSearchResult {
            key: "rec-1".to_string(),
            score: 0.95,
            value: None,
            vector: Some(vec![1u8, 2, 3, 4]),
        };
        let rt = roundtrip_named(&result);
        assert_eq!(rt, result);
        assert_eq!(rt.vector, Some(vec![1u8, 2, 3, 4]));
    }

    #[test]
    fn vector_search_resp_payload_roundtrip() {
        let result = VectorSearchResult {
            key: "k1".to_string(),
            score: 0.9,
            value: None,
            vector: None,
        };
        let payload = VectorSearchRespPayload {
            id: "resp-1".to_string(),
            results: vec![result],
            total_candidates: 10,
            search_time_ms: 5,
            error: None,
        };
        assert_eq!(roundtrip_named(&payload), payload);
    }

    #[test]
    fn vector_search_resp_payload_error_roundtrip() {
        let payload = VectorSearchRespPayload {
            id: "resp-err".to_string(),
            results: vec![],
            total_candidates: 0,
            search_time_ms: 0,
            error: Some("no index".to_string()),
        };
        let rt = roundtrip_named(&payload);
        assert_eq!(rt, payload);
        assert_eq!(rt.error, Some("no index".to_string()));
        assert!(rt.results.is_empty());
    }

    #[test]
    fn query_vector_serialized_as_bin() {
        let msg = VectorSearchPayload {
            id: "bin-test".to_string(),
            map_name: "m".to_string(),
            index_name: None,
            query_vector: make_query_vector_2d(),
            k: 1,
            ef_search: None,
            options: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).unwrap();
        let val: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        let map = match val {
            rmpv::Value::Map(m) => m,
            other => panic!("expected Map, got {other:?}"),
        };
        let qv_entry = map.iter().find(|(k, _)| {
            if let rmpv::Value::String(s) = k {
                s.as_str() == Some("queryVector")
            } else {
                false
            }
        });
        let qv_val = qv_entry.expect("queryVector key not found");
        match &qv_val.1 {
            rmpv::Value::Binary(_) => {} // correct: MsgPack bin format
            other => panic!("queryVector should be Binary, got {other:?}"),
        }
    }

    #[test]
    fn decode_query_vector_valid() {
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        let payload = VectorSearchPayload {
            id: "x".to_string(),
            map_name: "m".to_string(),
            index_name: None,
            query_vector: bytes,
            k: 1,
            ef_search: None,
            options: None,
        };
        let result = payload.decode_query_vector(None).unwrap();
        assert_eq!(result, vec![1.0f32, 2.0f32]);
    }

    #[test]
    fn decode_query_vector_wrong_length_errors() {
        let payload = VectorSearchPayload {
            id: "x".to_string(),
            map_name: "m".to_string(),
            index_name: None,
            query_vector: vec![1, 2, 3, 4, 5], // 5 bytes, not multiple of 4
            k: 1,
            ef_search: None,
            options: None,
        };
        let err = payload.decode_query_vector(None).unwrap_err();
        assert!(err.contains("not a multiple of 4"), "error was: {err}");
    }

    #[test]
    fn decode_query_vector_dim_mismatch_errors() {
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        // 8 bytes = 2 dimensions, but we pass expected_dim = Some(4)
        let payload = VectorSearchPayload {
            id: "x".to_string(),
            map_name: "m".to_string(),
            index_name: None,
            query_vector: bytes,
            k: 1,
            ef_search: None,
            options: None,
        };
        let err = payload.decode_query_vector(Some(4)).unwrap_err();
        assert!(
            err.contains("does not match expected dimension"),
            "error was: {err}"
        );
    }
}
