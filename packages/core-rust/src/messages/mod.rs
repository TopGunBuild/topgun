//! Wire-compatible message schemas for the `TopGun` protocol.
//!
//! Each submodule corresponds to a domain of messages defined in the TypeScript
//! Zod schemas (`packages/core/src/schemas/`). All types use named `MsgPack`
//! serialization (`rmp_serde::to_vec_named()`) with camelCase field names to
//! match the TypeScript wire format.

pub mod base;

pub mod cluster;
pub mod query;
pub mod search;
pub mod sync;

// Future submodules (SPEC-052d through SPEC-052e):
// pub mod messaging;
// pub mod client_events;
// pub mod http_sync;

pub use base::{
    AuthMessage, AuthRequiredMessage, ChangeEventType, ClientOp, PredicateNode, PredicateOp,
    Query, SortDirection, WriteConcern,
};

pub use cluster::{
    ClusterSearchReqOptions, ClusterSearchReqPayload, ClusterSearchRespPayload,
    ClusterSearchResultEntry, ClusterSearchSubscribePayload, ClusterSearchUnsubscribePayload,
    ClusterSearchUpdatePayload, ClusterSubAckPayload, ClusterSubAckResultEntry,
    ClusterSubRegisterPayload, ClusterSubType, ClusterSubUnregisterPayload,
    ClusterSubUpdatePayload, NodeEndpoints, NodeInfo, NodeStatus, PartitionInfo,
    PartitionMapPayload, PartitionMapRequestPayload,
};

pub use query::{
    CursorStatus, QueryRespMessage, QueryRespPayload, QueryResultEntry, QuerySubMessage,
    QuerySubPayload, QueryUnsubMessage, QueryUnsubPayload,
};

pub use search::{
    SearchOptions, SearchPayload, SearchRespPayload, SearchResultEntry, SearchSubPayload,
    SearchUnsubPayload, SearchUpdatePayload,
};

pub use sync::{
    BatchMessage, ClientOpMessage, MerkleReqBucketMessage, MerkleReqBucketPayload, OpAckMessage,
    OpAckPayload, OpBatchMessage, OpBatchPayload, OpRejectedMessage, OpRejectedPayload, OpResult,
    ORMapDiffRequest, ORMapDiffRequestPayload, ORMapDiffResponse, ORMapDiffResponsePayload,
    ORMapEntry, ORMapMerkleReqBucket, ORMapMerkleReqBucketPayload, ORMapPushDiff,
    ORMapPushDiffPayload, ORMapSyncInit, ORMapSyncRespBuckets, ORMapSyncRespBucketsPayload,
    ORMapSyncRespLeaf, ORMapSyncRespLeafPayload, ORMapSyncRespRoot, ORMapSyncRespRootPayload,
    SyncInitMessage, SyncLeafRecord, SyncRespBucketsMessage, SyncRespBucketsPayload,
    SyncRespLeafMessage, SyncRespLeafPayload, SyncRespRootMessage, SyncRespRootPayload,
};

#[cfg(test)]
mod prototype_tests {
    //! Prototype tests validating that `rmp_serde` supports internally-tagged enums
    //! with `MsgPack` named maps, and that Rust integer types produce `MsgPack` integer
    //! format (not float64). These tests gate the architectural approach for the
    //! future `Message` enum.

    use serde::{Deserialize, Serialize};

    use super::sync::OpAckPayload;

    /// Prototype of the future `Message` enum using `#[serde(tag = "type")]`.
    /// Tests three representative variant patterns to validate `rmp_serde` support.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type")]
    enum MessagePrototype {
        /// Payload-wrapped variant: inner struct nested under a `payload` key.
        #[serde(rename = "OP_ACK")]
        OpAck { payload: OpAckPayload },

        /// Flat variant: fields inlined directly into the message map.
        #[serde(rename = "SYNC_INIT")]
        SyncInit {
            #[serde(rename = "mapName")]
            map_name: String,
            #[serde(skip_serializing_if = "Option::is_none", default)]
            #[serde(rename = "lastSyncTimestamp")]
            last_sync_timestamp: Option<u64>,
        },

        /// Flat variant with binary data.
        #[serde(rename = "BATCH")]
        Batch {
            count: u32,
            #[serde(with = "serde_bytes")]
            data: Vec<u8>,
        },
    }

    /// Helper struct for integer wire format verification.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct IntegerWireTest {
        hash_value: u32,
        timestamp_ms: u64,
    }

    // ---- Prototype round-trip tests ----

    #[test]
    fn prototype_roundtrip_payload_wrapped() {
        let msg = MessagePrototype::OpAck {
            payload: OpAckPayload {
                last_id: "op-42".into(),
                achieved_level: None,
                results: None,
            },
        };

        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize OpAck");
        let decoded: MessagePrototype =
            rmp_serde::from_slice(&bytes).expect("deserialize OpAck");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn prototype_roundtrip_flat() {
        let msg = MessagePrototype::SyncInit {
            map_name: "users".into(),
            last_sync_timestamp: Some(1_700_000_000_000),
        };

        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize SyncInit");
        let decoded: MessagePrototype =
            rmp_serde::from_slice(&bytes).expect("deserialize SyncInit");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn prototype_roundtrip_flat_no_optional() {
        let msg = MessagePrototype::SyncInit {
            map_name: "orders".into(),
            last_sync_timestamp: None,
        };

        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize SyncInit without optional");
        let decoded: MessagePrototype =
            rmp_serde::from_slice(&bytes).expect("deserialize SyncInit without optional");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn prototype_roundtrip_flat_with_binary() {
        let msg = MessagePrototype::Batch {
            count: 5,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };

        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize Batch");
        let decoded: MessagePrototype =
            rmp_serde::from_slice(&bytes).expect("deserialize Batch");
        assert_eq!(msg, decoded);
    }

    // ---- Discriminator verification tests ----

    #[test]
    fn prototype_serialized_contains_type_discriminator() {
        // Verify each variant's serialized form contains exactly one "type" key
        // with the correct discriminator string.
        let cases: Vec<(MessagePrototype, &str)> = vec![
            (
                MessagePrototype::OpAck {
                    payload: OpAckPayload {
                        last_id: "x".into(),
                        achieved_level: None,
                        results: None,
                    },
                },
                "OP_ACK",
            ),
            (
                MessagePrototype::SyncInit {
                    map_name: "m".into(),
                    last_sync_timestamp: None,
                },
                "SYNC_INIT",
            ),
            (
                MessagePrototype::Batch {
                    count: 1,
                    data: vec![0xFF],
                },
                "BATCH",
            ),
        ];

        for (msg, expected_tag) in cases {
            let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");

            // Deserialize into rmpv::Value to inspect the raw MsgPack structure
            let value: rmpv::Value =
                rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");

            let map = value.as_map().expect("top-level should be a MsgPack map");

            // Count occurrences of the "type" key -- must be exactly 1
            let type_entries: Vec<_> = map
                .iter()
                .filter(|(k, _)| k.as_str() == Some("type"))
                .collect();
            assert_eq!(
                type_entries.len(),
                1,
                "expected exactly one 'type' key, found {}",
                type_entries.len()
            );

            // Verify the discriminator value
            let (_, tag_value) = type_entries[0];
            assert_eq!(
                tag_value.as_str(),
                Some(expected_tag),
                "expected tag '{expected_tag}', got {tag_value:?}",
            );
        }
    }

    #[test]
    fn prototype_dispatch_from_msgpack_map() {
        // Build a MsgPack map manually with "type" key and verify deserialization
        // dispatches to the correct variant.
        let msg = MessagePrototype::Batch {
            count: 3,
            data: vec![1, 2, 3],
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");

        // Deserialize as the enum -- serde should dispatch based on "type" key
        let decoded: MessagePrototype =
            rmp_serde::from_slice(&bytes).expect("dispatch from MsgPack map");
        assert_eq!(decoded, msg);
    }

    // ---- Integer wire format tests ----

    #[test]
    fn u32_serializes_as_msgpack_integer_not_float() {
        // Rust u32 must produce MsgPack integer format, not float64.
        // This is critical: TS msgpackr encodes integer-valued numbers as
        // MsgPack integers, so Rust must match that wire format.
        let val = IntegerWireTest {
            hash_value: 2_863_311_530, // 0xAAAAAAAA -- large u32
            timestamp_ms: 1_700_000_000_000,
        };

        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");

        let map = raw.as_map().expect("should be map");
        for (key, value) in map {
            let field_name = key.as_str().unwrap_or("?");
            assert!(
                matches!(value, rmpv::Value::Integer(_)),
                "field '{field_name}' should be MsgPack Integer, got {value:?}",
            );
            assert!(
                !matches!(value, rmpv::Value::F64(_) | rmpv::Value::F32(_)),
                "field '{field_name}' must NOT be MsgPack Float",
            );
        }
    }

    #[test]
    fn u64_serializes_as_msgpack_integer_not_float() {
        // Same verification for u64 specifically
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct U64Test {
            big_timestamp: u64,
        }

        let val = U64Test {
            big_timestamp: u64::MAX,
        };

        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");

        let map = raw.as_map().expect("should be map");
        let (_, value) = &map[0];
        assert!(
            matches!(value, rmpv::Value::Integer(_)),
            "u64::MAX should be MsgPack Integer, got {value:?}",
        );
    }

    #[test]
    fn u32_zero_serializes_as_msgpack_integer() {
        // Edge case: zero should still be integer format
        #[derive(Serialize)]
        struct ZeroTest {
            val: u32,
        }

        let bytes = rmp_serde::to_vec_named(&ZeroTest { val: 0 }).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");

        let map = raw.as_map().expect("should be map");
        let (_, value) = &map[0];
        assert!(
            matches!(value, rmpv::Value::Integer(_)),
            "u32 zero should be MsgPack Integer, got {value:?}",
        );
    }
}

#[cfg(test)]
mod search_tests {
    //! Round-trip serde tests for search domain payload structs.

    use std::collections::HashMap;

    use super::base::ChangeEventType;
    use super::search::{
        SearchOptions, SearchPayload, SearchRespPayload, SearchResultEntry, SearchSubPayload,
        SearchUnsubPayload, SearchUpdatePayload,
    };

    #[test]
    fn search_options_roundtrip_all_fields() {
        let mut boost = HashMap::new();
        boost.insert("title".into(), 2.0);
        boost.insert("body".into(), 1.0);

        let opts = SearchOptions {
            limit: Some(50),
            min_score: Some(0.5),
            boost: Some(boost),
        };

        let bytes = rmp_serde::to_vec_named(&opts).expect("serialize");
        let decoded: SearchOptions = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(opts, decoded);
    }

    #[test]
    fn search_options_roundtrip_default() {
        let opts = SearchOptions::default();
        let bytes = rmp_serde::to_vec_named(&opts).expect("serialize");
        let decoded: SearchOptions = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(opts, decoded);
    }

    #[test]
    fn search_payload_roundtrip() {
        let payload = SearchPayload {
            request_id: "req-001".into(),
            map_name: "articles".into(),
            query: "rust async".into(),
            options: Some(SearchOptions {
                limit: Some(10),
                min_score: None,
                boost: None,
            }),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_payload_roundtrip_no_options() {
        let payload = SearchPayload {
            request_id: "req-002".into(),
            map_name: "users".into(),
            query: "admin".into(),
            options: None,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_resp_payload_roundtrip() {
        let payload = SearchRespPayload {
            request_id: "req-001".into(),
            results: vec![SearchResultEntry {
                key: "doc-42".into(),
                value: rmpv::Value::Map(vec![(
                    rmpv::Value::String("title".into()),
                    rmpv::Value::String("Async Rust".into()),
                )]),
                score: 0.95,
                matched_terms: vec!["rust".into(), "async".into()],
            }],
            total_count: 1,
            error: None,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchRespPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_resp_payload_roundtrip_with_error() {
        let payload = SearchRespPayload {
            request_id: "req-err".into(),
            results: vec![],
            total_count: 0,
            error: Some("index not found".into()),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchRespPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_sub_payload_roundtrip() {
        let payload = SearchSubPayload {
            subscription_id: "sub-001".into(),
            map_name: "articles".into(),
            query: "machine learning".into(),
            options: Some(SearchOptions {
                limit: Some(20),
                min_score: Some(0.3),
                boost: None,
            }),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchSubPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_update_payload_roundtrip() {
        let payload = SearchUpdatePayload {
            subscription_id: "sub-001".into(),
            key: "doc-99".into(),
            value: rmpv::Value::String("updated content".into()),
            score: 0.88,
            matched_terms: vec!["learning".into()],
            change_type: ChangeEventType::UPDATE,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchUpdatePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_update_payload_roundtrip_enter() {
        let payload = SearchUpdatePayload {
            subscription_id: "sub-002".into(),
            key: "new-doc".into(),
            value: rmpv::Value::Nil,
            score: 0.75,
            matched_terms: vec!["rust".into()],
            change_type: ChangeEventType::ENTER,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchUpdatePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn search_unsub_payload_roundtrip() {
        let payload = SearchUnsubPayload {
            subscription_id: "sub-001".into(),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SearchUnsubPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }
}

#[cfg(test)]
mod cluster_tests {
    //! Round-trip serde tests for cluster domain payload structs.

    use std::collections::HashMap;

    use super::base::{ChangeEventType, SortDirection};
    use super::cluster::{
        ClusterSearchReqOptions, ClusterSearchReqPayload, ClusterSearchRespPayload,
        ClusterSearchResultEntry, ClusterSearchSubscribePayload,
        ClusterSearchUnsubscribePayload, ClusterSearchUpdatePayload, ClusterSubAckPayload,
        ClusterSubAckResultEntry, ClusterSubRegisterPayload, ClusterSubType,
        ClusterSubUnregisterPayload, ClusterSubUpdatePayload, NodeEndpoints, NodeInfo,
        NodeStatus, PartitionInfo, PartitionMapPayload, PartitionMapRequestPayload,
    };
    use super::search::SearchOptions;

    // ---- Partition Map types ----

    #[test]
    fn node_status_roundtrip() {
        for status in [
            NodeStatus::ACTIVE,
            NodeStatus::JOINING,
            NodeStatus::LEAVING,
            NodeStatus::SUSPECTED,
            NodeStatus::FAILED,
        ] {
            let bytes = rmp_serde::to_vec_named(&status).expect("serialize");
            let decoded: NodeStatus = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(status, decoded);
        }
    }

    #[test]
    fn node_endpoints_roundtrip() {
        let endpoints = NodeEndpoints {
            websocket: "ws://10.0.0.1:8080".into(),
            http: Some("http://10.0.0.1:3000".into()),
        };

        let bytes = rmp_serde::to_vec_named(&endpoints).expect("serialize");
        let decoded: NodeEndpoints = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(endpoints, decoded);
    }

    #[test]
    fn node_endpoints_roundtrip_no_http() {
        let endpoints = NodeEndpoints {
            websocket: "ws://10.0.0.1:8080".into(),
            http: None,
        };

        let bytes = rmp_serde::to_vec_named(&endpoints).expect("serialize");
        let decoded: NodeEndpoints = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(endpoints, decoded);
    }

    #[test]
    fn node_info_roundtrip() {
        let info = NodeInfo {
            node_id: "node-1".into(),
            endpoints: NodeEndpoints {
                websocket: "ws://10.0.0.1:8080".into(),
                http: None,
            },
            status: NodeStatus::ACTIVE,
        };

        let bytes = rmp_serde::to_vec_named(&info).expect("serialize");
        let decoded: NodeInfo = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(info, decoded);
    }

    #[test]
    fn partition_info_roundtrip() {
        let info = PartitionInfo {
            partition_id: 42,
            owner_node_id: "node-1".into(),
            backup_node_ids: vec!["node-2".into(), "node-3".into()],
        };

        let bytes = rmp_serde::to_vec_named(&info).expect("serialize");
        let decoded: PartitionInfo = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(info, decoded);
    }

    #[test]
    fn partition_map_payload_roundtrip() {
        let payload = PartitionMapPayload {
            version: 7,
            partition_count: 271,
            nodes: vec![NodeInfo {
                node_id: "node-1".into(),
                endpoints: NodeEndpoints {
                    websocket: "ws://10.0.0.1:8080".into(),
                    http: Some("http://10.0.0.1:3000".into()),
                },
                status: NodeStatus::ACTIVE,
            }],
            partitions: vec![PartitionInfo {
                partition_id: 0,
                owner_node_id: "node-1".into(),
                backup_node_ids: vec![],
            }],
            generated_at: 1_700_000_000_000,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: PartitionMapPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn partition_map_request_payload_roundtrip() {
        let payload = PartitionMapRequestPayload {
            current_version: Some(5),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: PartitionMapRequestPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn partition_map_request_payload_roundtrip_none() {
        let payload = PartitionMapRequestPayload {
            current_version: None,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: PartitionMapRequestPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- ClusterSub types ----

    #[test]
    fn cluster_sub_type_roundtrip() {
        for sub_type in [ClusterSubType::SEARCH, ClusterSubType::QUERY] {
            let bytes = rmp_serde::to_vec_named(&sub_type).expect("serialize");
            let decoded: ClusterSubType = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(sub_type, decoded);
        }
    }

    #[test]
    fn cluster_sub_register_payload_roundtrip_search() {
        let mut boost = HashMap::new();
        boost.insert("title".into(), 2.5);

        let payload = ClusterSubRegisterPayload {
            subscription_id: "csub-001".into(),
            coordinator_node_id: "node-1".into(),
            map_name: "articles".into(),
            sub_type: ClusterSubType::SEARCH,
            search_query: Some("rust async".into()),
            search_options: Some(SearchOptions {
                limit: Some(10),
                min_score: Some(0.5),
                boost: Some(boost),
            }),
            query_predicate: None,
            query_sort: None,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSubRegisterPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_sub_register_payload_roundtrip_query() {
        let mut sort = HashMap::new();
        sort.insert("createdAt".into(), SortDirection::Desc);

        let payload = ClusterSubRegisterPayload {
            subscription_id: "csub-002".into(),
            coordinator_node_id: "node-2".into(),
            map_name: "users".into(),
            sub_type: ClusterSubType::QUERY,
            search_query: None,
            search_options: None,
            query_predicate: Some(rmpv::Value::String("predicate-tree".into())),
            query_sort: Some(sort),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSubRegisterPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_sub_register_type_field_serializes_as_type() {
        // AC-type-field: verify the sub_type field serializes as "type" in the MsgPack map
        let payload = ClusterSubRegisterPayload {
            subscription_id: "csub-t".into(),
            coordinator_node_id: "node-1".into(),
            map_name: "test".into(),
            sub_type: ClusterSubType::SEARCH,
            ..Default::default()
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");

        // Find the "type" key in the serialized map
        let type_entry = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("type"))
            .expect("should have a 'type' key");

        assert_eq!(
            type_entry.1.as_str(),
            Some("SEARCH"),
            "sub_type field should serialize as 'type' with value 'SEARCH'"
        );
    }

    #[test]
    fn cluster_sub_ack_result_entry_roundtrip() {
        let entry = ClusterSubAckResultEntry {
            key: "doc-1".into(),
            value: rmpv::Value::String("content".into()),
            score: Some(0.9),
            matched_terms: Some(vec!["rust".into()]),
        };

        let bytes = rmp_serde::to_vec_named(&entry).expect("serialize");
        let decoded: ClusterSubAckResultEntry =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(entry, decoded);
    }

    #[test]
    fn cluster_sub_ack_payload_roundtrip() {
        let payload = ClusterSubAckPayload {
            subscription_id: "csub-001".into(),
            node_id: "node-2".into(),
            success: true,
            error: None,
            initial_results: Some(vec![ClusterSubAckResultEntry {
                key: "doc-1".into(),
                value: rmpv::Value::Nil,
                score: None,
                matched_terms: None,
            }]),
            total_hits: Some(42),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSubAckPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_sub_ack_payload_roundtrip_failure() {
        let payload = ClusterSubAckPayload {
            subscription_id: "csub-err".into(),
            node_id: "node-3".into(),
            success: false,
            error: Some("map not found".into()),
            initial_results: None,
            total_hits: None,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSubAckPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_sub_update_payload_roundtrip() {
        let payload = ClusterSubUpdatePayload {
            subscription_id: "csub-001".into(),
            source_node_id: "node-2".into(),
            key: "doc-5".into(),
            value: rmpv::Value::String("new value".into()),
            score: Some(0.8),
            matched_terms: Some(vec!["async".into()]),
            change_type: ChangeEventType::ENTER,
            timestamp: 1_700_000_000_001,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSubUpdatePayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_sub_unregister_payload_roundtrip() {
        let payload = ClusterSubUnregisterPayload {
            subscription_id: "csub-001".into(),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSubUnregisterPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- ClusterSearch types ----

    #[test]
    fn cluster_search_req_options_roundtrip() {
        let opts = ClusterSearchReqOptions {
            limit: 25,
            min_score: Some(0.3),
            boost: None,
            include_matched_terms: Some(true),
            after_score: Some(0.75),
            after_key: Some("last-key".into()),
        };

        let bytes = rmp_serde::to_vec_named(&opts).expect("serialize");
        let decoded: ClusterSearchReqOptions =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(opts, decoded);
    }

    #[test]
    fn cluster_search_req_payload_roundtrip() {
        let payload = ClusterSearchReqPayload {
            request_id: "csearch-001".into(),
            map_name: "products".into(),
            query: "laptop".into(),
            options: ClusterSearchReqOptions {
                limit: 10,
                min_score: None,
                boost: None,
                include_matched_terms: None,
                after_score: None,
                after_key: None,
            },
            timeout_ms: Some(5000),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchReqPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_search_resp_payload_roundtrip() {
        let payload = ClusterSearchRespPayload {
            request_id: "csearch-001".into(),
            node_id: "node-3".into(),
            results: vec![ClusterSearchResultEntry {
                key: "prod-1".into(),
                value: rmpv::Value::String("Laptop Pro".into()),
                score: 0.92,
                matched_terms: Some(vec!["laptop".into()]),
            }],
            total_hits: 150,
            execution_time_ms: 23,
            error: None,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchRespPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_search_resp_payload_roundtrip_with_error() {
        let payload = ClusterSearchRespPayload {
            request_id: "csearch-err".into(),
            node_id: "node-4".into(),
            results: vec![],
            total_hits: 0,
            execution_time_ms: 1,
            error: Some("timeout exceeded".into()),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchRespPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_search_subscribe_payload_roundtrip() {
        let payload = ClusterSearchSubscribePayload {
            subscription_id: "css-001".into(),
            map_name: "products".into(),
            query: "monitor".into(),
            options: Some(SearchOptions {
                limit: Some(5),
                min_score: None,
                boost: None,
            }),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchSubscribePayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_search_unsubscribe_payload_roundtrip() {
        let payload = ClusterSearchUnsubscribePayload {
            subscription_id: "css-001".into(),
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchUnsubscribePayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_search_update_payload_roundtrip() {
        let payload = ClusterSearchUpdatePayload {
            subscription_id: "css-001".into(),
            node_id: "node-2".into(),
            key: "prod-99".into(),
            value: rmpv::Value::String("Updated Monitor".into()),
            score: 0.85,
            matched_terms: Some(vec!["monitor".into()]),
            change_type: ChangeEventType::UPDATE,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchUpdatePayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn cluster_search_update_payload_roundtrip_leave() {
        let payload = ClusterSearchUpdatePayload {
            subscription_id: "css-002".into(),
            node_id: "node-1".into(),
            key: "prod-50".into(),
            value: rmpv::Value::Nil,
            score: 0.0,
            matched_terms: None,
            change_type: ChangeEventType::LEAVE,
        };

        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ClusterSearchUpdatePayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }
}
