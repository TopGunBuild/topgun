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

pub mod client_events;
pub mod messaging;

// Future submodules (SPEC-052e):
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

pub use client_events::{
    AuthAckData, AuthFailData, ErrorPayload, GcPrunePayload, LockGrantedPayload,
    LockReleasedPayload, QueryUpdatePayload, ServerBatchEventPayload, ServerEventPayload,
    ServerEventType, SyncResetRequiredPayload,
};

pub use messaging::{
    ConflictResolver, CounterRequestPayload, CounterStatePayload, EntryProcessBatchData,
    EntryProcessBatchResponseData, EntryProcessData, EntryProcessKeyResult, EntryProcessor,
    EntryProcessResponseData, JournalEventData, JournalEventMessageData, JournalEventType,
    JournalReadData, JournalReadResponseData, JournalSubscribeData, JournalUnsubscribeData,
    ListResolversData, ListResolversResponseData, LockReleasePayload, LockRequestPayload,
    MergeRejectedData, PingData, PNCounterState, PongData, RegisterResolverData,
    RegisterResolverResponseData, ResolverInfo, TopicMessageEventPayload, TopicPubPayload,
    TopicSubPayload, TopicUnsubPayload, UnregisterResolverData, UnregisterResolverResponseData,
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

#[cfg(test)]
mod messaging_tests {
    //! Round-trip serde tests for messaging domain payload structs.

    use std::collections::HashMap;

    use crate::hlc::Timestamp;

    use super::messaging::{
        ConflictResolver, CounterRequestPayload, CounterStatePayload, EntryProcessBatchData,
        EntryProcessBatchResponseData, EntryProcessData, EntryProcessKeyResult, EntryProcessor,
        EntryProcessResponseData, JournalEventData, JournalEventMessageData, JournalEventType,
        JournalReadData, JournalReadResponseData, JournalSubscribeData, JournalUnsubscribeData,
        ListResolversData, ListResolversResponseData, LockReleasePayload, LockRequestPayload,
        MergeRejectedData, PNCounterState, PingData, PongData, RegisterResolverData,
        RegisterResolverResponseData, ResolverInfo, TopicMessageEventPayload, TopicPubPayload,
        TopicSubPayload, TopicUnsubPayload, UnregisterResolverData,
        UnregisterResolverResponseData,
    };

    // ---- Topic payloads ----

    #[test]
    fn topic_sub_payload_roundtrip() {
        let payload = TopicSubPayload {
            topic: "chat/room-1".into(),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: TopicSubPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn topic_unsub_payload_roundtrip() {
        let payload = TopicUnsubPayload {
            topic: "chat/room-1".into(),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: TopicUnsubPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn topic_pub_payload_roundtrip() {
        let payload = TopicPubPayload {
            topic: "notifications".into(),
            data: rmpv::Value::String("hello world".into()),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: TopicPubPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn topic_message_event_payload_roundtrip() {
        let payload = TopicMessageEventPayload {
            topic: "chat/room-1".into(),
            data: rmpv::Value::Map(vec![(
                rmpv::Value::String("text".into()),
                rmpv::Value::String("Hi there".into()),
            )]),
            publisher_id: Some("user-42".into()),
            timestamp: 1_700_000_000_000,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: TopicMessageEventPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn topic_message_event_payload_roundtrip_no_publisher() {
        let payload = TopicMessageEventPayload {
            topic: "events".into(),
            data: rmpv::Value::Integer(42.into()),
            publisher_id: None,
            timestamp: 1_700_000_000_001,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: TopicMessageEventPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- Lock payloads ----

    #[test]
    fn lock_request_payload_roundtrip() {
        let payload = LockRequestPayload {
            request_id: "lock-req-1".into(),
            name: "my-lock".into(),
            ttl: Some(30_000),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockRequestPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn lock_request_payload_roundtrip_no_ttl() {
        let payload = LockRequestPayload {
            request_id: "lock-req-2".into(),
            name: "infinite-lock".into(),
            ttl: None,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockRequestPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn lock_release_payload_roundtrip() {
        let payload = LockReleasePayload {
            request_id: Some("lock-req-1".into()),
            name: "my-lock".into(),
            fencing_token: 7,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockReleasePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn lock_release_payload_roundtrip_no_request_id() {
        let payload = LockReleasePayload {
            request_id: None,
            name: "my-lock".into(),
            fencing_token: 12,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockReleasePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- PN Counter ----

    #[test]
    fn pn_counter_state_roundtrip() {
        let mut p = HashMap::new();
        p.insert("node-1".into(), 5.0);
        p.insert("node-2".into(), 3.5);
        let mut n = HashMap::new();
        n.insert("node-1".into(), 1.0);

        let state = PNCounterState { p, n };
        let bytes = rmp_serde::to_vec_named(&state).expect("serialize");
        let decoded: PNCounterState = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(state, decoded);
    }

    #[test]
    fn counter_request_payload_roundtrip() {
        let payload = CounterRequestPayload {
            name: "page-views".into(),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: CounterRequestPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn counter_state_payload_roundtrip() {
        let mut p = HashMap::new();
        p.insert("node-1".into(), 10.0);
        let n = HashMap::new();

        let payload = CounterStatePayload {
            name: "page-views".into(),
            state: PNCounterState { p, n },
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: CounterStatePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- Heartbeat ----

    #[test]
    fn ping_data_roundtrip() {
        let data = PingData {
            timestamp: 1_700_000_000_000,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: PingData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn pong_data_roundtrip() {
        let data = PongData {
            timestamp: 1_700_000_000_000,
            server_time: 1_700_000_000_005,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: PongData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    // ---- Entry Processor ----

    #[test]
    fn entry_processor_roundtrip() {
        let proc = EntryProcessor {
            name: "increment".into(),
            code: "return value + 1".into(),
            args: Some(rmpv::Value::Integer(1.into())),
        };
        let bytes = rmp_serde::to_vec_named(&proc).expect("serialize");
        let decoded: EntryProcessor = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(proc, decoded);
    }

    #[test]
    fn entry_processor_roundtrip_no_args() {
        let proc = EntryProcessor {
            name: "reset".into(),
            code: "return 0".into(),
            args: None,
        };
        let bytes = rmp_serde::to_vec_named(&proc).expect("serialize");
        let decoded: EntryProcessor = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(proc, decoded);
    }

    #[test]
    fn entry_process_data_roundtrip() {
        let data = EntryProcessData {
            request_id: "ep-1".into(),
            map_name: "users".into(),
            key: "user-1".into(),
            processor: EntryProcessor {
                name: "inc".into(),
                code: "return v+1".into(),
                args: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: EntryProcessData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn entry_process_batch_data_roundtrip() {
        let data = EntryProcessBatchData {
            request_id: "epb-1".into(),
            map_name: "counters".into(),
            keys: vec!["a".into(), "b".into(), "c".into()],
            processor: EntryProcessor {
                name: "reset".into(),
                code: "return 0".into(),
                args: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: EntryProcessBatchData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn entry_process_key_result_roundtrip() {
        let result = EntryProcessKeyResult {
            success: true,
            result: Some(rmpv::Value::Integer(42.into())),
            new_value: Some(rmpv::Value::Integer(42.into())),
            error: None,
        };
        let bytes = rmp_serde::to_vec_named(&result).expect("serialize");
        let decoded: EntryProcessKeyResult = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(result, decoded);
    }

    #[test]
    fn entry_process_key_result_roundtrip_failure() {
        let result = EntryProcessKeyResult {
            success: false,
            result: None,
            new_value: None,
            error: Some("timeout".into()),
        };
        let bytes = rmp_serde::to_vec_named(&result).expect("serialize");
        let decoded: EntryProcessKeyResult = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(result, decoded);
    }

    #[test]
    fn entry_process_response_data_roundtrip() {
        let data = EntryProcessResponseData {
            request_id: "ep-1".into(),
            success: true,
            result: Some(rmpv::Value::Integer(99.into())),
            new_value: Some(rmpv::Value::Integer(99.into())),
            error: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: EntryProcessResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn entry_process_batch_response_data_roundtrip() {
        let mut results = HashMap::new();
        results.insert(
            "key-a".into(),
            EntryProcessKeyResult {
                success: true,
                result: Some(rmpv::Value::Integer(1.into())),
                new_value: None,
                error: None,
            },
        );
        results.insert(
            "key-b".into(),
            EntryProcessKeyResult {
                success: false,
                result: None,
                new_value: None,
                error: Some("not found".into()),
            },
        );

        let data = EntryProcessBatchResponseData {
            request_id: "epb-1".into(),
            results,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: EntryProcessBatchResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    // ---- Journal ----

    #[test]
    fn journal_event_type_roundtrip() {
        for evt_type in [
            JournalEventType::PUT,
            JournalEventType::UPDATE,
            JournalEventType::DELETE,
        ] {
            let bytes = rmp_serde::to_vec_named(&evt_type).expect("serialize");
            let decoded: JournalEventType = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(evt_type, decoded);
        }
    }

    #[test]
    fn journal_event_data_roundtrip() {
        let data = JournalEventData {
            sequence: "seq-001".into(),
            event_type: JournalEventType::PUT,
            map_name: "users".into(),
            key: "user-1".into(),
            value: Some(rmpv::Value::String("Alice".into())),
            previous_value: None,
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 1,
                node_id: "node-1".into(),
            },
            node_id: "node-1".into(),
            metadata: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalEventData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_event_data_roundtrip_with_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("source".into(), rmpv::Value::String("api".into()));
        metadata.insert("version".into(), rmpv::Value::Integer(2.into()));

        let data = JournalEventData {
            sequence: "seq-002".into(),
            event_type: JournalEventType::UPDATE,
            map_name: "users".into(),
            key: "user-1".into(),
            value: Some(rmpv::Value::String("Bob".into())),
            previous_value: Some(rmpv::Value::String("Alice".into())),
            timestamp: Timestamp {
                millis: 1_700_000_000_001,
                counter: 0,
                node_id: "node-2".into(),
            },
            node_id: "node-2".into(),
            metadata: Some(metadata),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalEventData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_event_data_type_field_serializes_as_type() {
        // AC-journal-type-field: event_type must serialize as "type"
        let data = JournalEventData {
            sequence: "seq-t".into(),
            event_type: JournalEventType::DELETE,
            map_name: "m".into(),
            key: "k".into(),
            timestamp: Timestamp {
                millis: 0,
                counter: 0,
                node_id: "n".into(),
            },
            node_id: "n".into(),
            ..Default::default()
        };

        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");

        let type_entry = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("type"))
            .expect("should have a 'type' key");

        assert_eq!(
            type_entry.1.as_str(),
            Some("DELETE"),
            "event_type field should serialize as 'type' with value 'DELETE'"
        );
    }

    #[test]
    fn journal_event_data_default() {
        let data = JournalEventData::default();
        assert_eq!(data.event_type, JournalEventType::PUT);
        assert_eq!(data.timestamp.millis, 0);
        assert_eq!(data.timestamp.counter, 0);
        assert!(data.timestamp.node_id.is_empty());
        assert!(data.value.is_none());
        assert!(data.metadata.is_none());
    }

    #[test]
    fn journal_subscribe_data_roundtrip() {
        let data = JournalSubscribeData {
            request_id: "jsub-1".into(),
            from_sequence: Some("seq-100".into()),
            map_name: Some("users".into()),
            types: Some(vec![JournalEventType::PUT, JournalEventType::DELETE]),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalSubscribeData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_subscribe_data_roundtrip_minimal() {
        let data = JournalSubscribeData {
            request_id: "jsub-2".into(),
            from_sequence: None,
            map_name: None,
            types: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalSubscribeData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_unsubscribe_data_roundtrip() {
        let data = JournalUnsubscribeData {
            subscription_id: "jsub-1".into(),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalUnsubscribeData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_event_message_data_roundtrip() {
        let data = JournalEventMessageData {
            event: JournalEventData {
                sequence: "seq-050".into(),
                event_type: JournalEventType::UPDATE,
                map_name: "orders".into(),
                key: "order-7".into(),
                value: Some(rmpv::Value::String("shipped".into())),
                previous_value: Some(rmpv::Value::String("pending".into())),
                timestamp: Timestamp {
                    millis: 1_700_000_000_100,
                    counter: 3,
                    node_id: "node-1".into(),
                },
                node_id: "node-1".into(),
                metadata: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalEventMessageData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_read_data_roundtrip() {
        let data = JournalReadData {
            request_id: "jread-1".into(),
            from_sequence: "seq-000".into(),
            limit: Some(100),
            map_name: Some("users".into()),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalReadData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_read_data_roundtrip_minimal() {
        let data = JournalReadData {
            request_id: "jread-2".into(),
            from_sequence: "seq-500".into(),
            limit: None,
            map_name: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalReadData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_read_response_data_roundtrip() {
        let data = JournalReadResponseData {
            request_id: "jread-1".into(),
            events: vec![JournalEventData {
                sequence: "seq-001".into(),
                event_type: JournalEventType::PUT,
                map_name: "users".into(),
                key: "u1".into(),
                value: Some(rmpv::Value::String("v".into())),
                previous_value: None,
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 0,
                    node_id: "n1".into(),
                },
                node_id: "n1".into(),
                metadata: None,
            }],
            has_more: true,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalReadResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn journal_read_response_data_roundtrip_empty() {
        let data = JournalReadResponseData {
            request_id: "jread-3".into(),
            events: vec![],
            has_more: false,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: JournalReadResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    // ---- Conflict Resolver ----

    #[test]
    fn conflict_resolver_roundtrip() {
        let resolver = ConflictResolver {
            name: "custom-merge".into(),
            code: "if (a > b) return a; else return b;".into(),
            priority: Some(50),
            key_pattern: Some("user-*".into()),
        };
        let bytes = rmp_serde::to_vec_named(&resolver).expect("serialize");
        let decoded: ConflictResolver = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(resolver, decoded);
    }

    #[test]
    fn conflict_resolver_roundtrip_minimal() {
        let resolver = ConflictResolver {
            name: "simple".into(),
            code: "return a".into(),
            priority: None,
            key_pattern: None,
        };
        let bytes = rmp_serde::to_vec_named(&resolver).expect("serialize");
        let decoded: ConflictResolver = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(resolver, decoded);
    }

    #[test]
    fn register_resolver_data_roundtrip() {
        let data = RegisterResolverData {
            request_id: "rr-1".into(),
            map_name: "users".into(),
            resolver: ConflictResolver {
                name: "merge".into(),
                code: "return newest".into(),
                priority: Some(10),
                key_pattern: None,
            },
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: RegisterResolverData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn register_resolver_response_data_roundtrip_success() {
        let data = RegisterResolverResponseData {
            request_id: "rr-1".into(),
            success: true,
            error: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: RegisterResolverResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn register_resolver_response_data_roundtrip_failure() {
        let data = RegisterResolverResponseData {
            request_id: "rr-1".into(),
            success: false,
            error: Some("code too long".into()),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: RegisterResolverResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn unregister_resolver_data_roundtrip() {
        let data = UnregisterResolverData {
            request_id: "ur-1".into(),
            map_name: "users".into(),
            resolver_name: "merge".into(),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: UnregisterResolverData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn unregister_resolver_response_data_roundtrip() {
        let data = UnregisterResolverResponseData {
            request_id: "ur-1".into(),
            success: true,
            error: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: UnregisterResolverResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn merge_rejected_data_roundtrip() {
        let data = MergeRejectedData {
            map_name: "users".into(),
            key: "user-1".into(),
            attempted_value: rmpv::Value::String("bad-value".into()),
            reason: "resolver rejected".into(),
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 5,
                node_id: "node-3".into(),
            },
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: MergeRejectedData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn resolver_info_roundtrip() {
        let info = ResolverInfo {
            map_name: "users".into(),
            name: "custom-merge".into(),
            priority: Some(50),
            key_pattern: Some("admin-*".into()),
        };
        let bytes = rmp_serde::to_vec_named(&info).expect("serialize");
        let decoded: ResolverInfo = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(info, decoded);
    }

    #[test]
    fn resolver_info_roundtrip_minimal() {
        let info = ResolverInfo {
            map_name: "orders".into(),
            name: "simple".into(),
            priority: None,
            key_pattern: None,
        };
        let bytes = rmp_serde::to_vec_named(&info).expect("serialize");
        let decoded: ResolverInfo = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(info, decoded);
    }

    #[test]
    fn list_resolvers_data_roundtrip() {
        let data = ListResolversData {
            request_id: "lr-1".into(),
            map_name: Some("users".into()),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: ListResolversData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn list_resolvers_data_roundtrip_no_filter() {
        let data = ListResolversData {
            request_id: "lr-2".into(),
            map_name: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: ListResolversData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn list_resolvers_response_data_roundtrip() {
        let data = ListResolversResponseData {
            request_id: "lr-1".into(),
            resolvers: vec![
                ResolverInfo {
                    map_name: "users".into(),
                    name: "merge-a".into(),
                    priority: Some(10),
                    key_pattern: None,
                },
                ResolverInfo {
                    map_name: "users".into(),
                    name: "merge-b".into(),
                    priority: Some(20),
                    key_pattern: Some("admin-*".into()),
                },
            ],
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: ListResolversResponseData =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    // ---- AC-no-type-field: verify no struct has a "type" key except JournalEventData ----

    #[test]
    fn messaging_flat_structs_have_no_type_key() {
        // Verify representative flat structs do NOT produce a "type" key
        let ping = PingData { timestamp: 100 };
        let bytes = rmp_serde::to_vec_named(&ping).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");
        let type_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("type"))
            .collect();
        assert!(
            type_keys.is_empty(),
            "PingData should not have a 'type' key"
        );

        let proc_data = EntryProcessResponseData {
            request_id: "x".into(),
            success: true,
            ..Default::default()
        };
        let bytes = rmp_serde::to_vec_named(&proc_data).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");
        let type_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("type"))
            .collect();
        assert!(
            type_keys.is_empty(),
            "EntryProcessResponseData should not have a 'type' key"
        );
    }
}

#[cfg(test)]
mod client_events_tests {
    //! Round-trip serde tests for client event domain payload structs.

    use crate::hlc::{LWWRecord, ORMapRecord, Timestamp};

    use super::base::ChangeEventType;
    use super::client_events::{
        AuthAckData, AuthFailData, ErrorPayload, GcPrunePayload, LockGrantedPayload,
        LockReleasedPayload, QueryUpdatePayload, ServerBatchEventPayload, ServerEventPayload,
        ServerEventType, SyncResetRequiredPayload,
    };

    // ---- ServerEventType ----

    #[test]
    fn server_event_type_roundtrip() {
        for evt in [
            ServerEventType::PUT,
            ServerEventType::REMOVE,
            ServerEventType::OR_ADD,
            ServerEventType::OR_REMOVE,
        ] {
            let bytes = rmp_serde::to_vec_named(&evt).expect("serialize");
            let decoded: ServerEventType = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(evt, decoded);
        }
    }

    #[test]
    fn server_event_type_is_distinct_from_change_event_type() {
        // AC-event-type-distinction: these are separate enums
        let server_put = rmp_serde::to_vec_named(&ServerEventType::PUT).expect("serialize");
        let change_enter = rmp_serde::to_vec_named(&ChangeEventType::ENTER).expect("serialize");

        let server_str: String = rmp_serde::from_slice(&server_put).expect("deserialize");
        let change_str: String = rmp_serde::from_slice(&change_enter).expect("deserialize");

        assert_eq!(server_str, "PUT");
        assert_eq!(change_str, "ENTER");
        assert_ne!(server_str, change_str);
    }

    // ---- ServerEventPayload ----

    #[test]
    fn server_event_payload_roundtrip_put() {
        let payload = ServerEventPayload {
            map_name: "users".into(),
            event_type: ServerEventType::PUT,
            key: "user-1".into(),
            record: Some(LWWRecord {
                value: Some(rmpv::Value::String("Alice".into())),
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 1,
                    node_id: "node-1".into(),
                },
                ttl_ms: None,
            }),
            or_record: None,
            or_tag: None,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ServerEventPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn server_event_payload_roundtrip_or_add() {
        let payload = ServerEventPayload {
            map_name: "tags".into(),
            event_type: ServerEventType::OR_ADD,
            key: "item-1".into(),
            record: None,
            or_record: Some(ORMapRecord {
                value: rmpv::Value::String("tag-a".into()),
                timestamp: Timestamp {
                    millis: 1_700_000_000_000,
                    counter: 0,
                    node_id: "node-2".into(),
                },
                tag: "1700000000000:0:node-2".into(),
                ttl_ms: None,
            }),
            or_tag: Some("1700000000000:0:node-2".into()),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ServerEventPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn server_event_payload_roundtrip_remove() {
        let payload = ServerEventPayload {
            map_name: "users".into(),
            event_type: ServerEventType::REMOVE,
            key: "user-2".into(),
            record: Some(LWWRecord {
                value: None,
                timestamp: Timestamp {
                    millis: 1_700_000_000_010,
                    counter: 0,
                    node_id: "node-1".into(),
                },
                ttl_ms: None,
            }),
            or_record: None,
            or_tag: None,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ServerEventPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn server_event_payload_default() {
        let payload = ServerEventPayload::default();
        assert_eq!(payload.event_type, ServerEventType::PUT);
        assert!(payload.record.is_none());
        assert!(payload.or_record.is_none());
        assert!(payload.or_tag.is_none());
    }

    // ---- ServerBatchEventPayload ----

    #[test]
    fn server_batch_event_payload_roundtrip() {
        let payload = ServerBatchEventPayload {
            events: vec![
                ServerEventPayload {
                    map_name: "users".into(),
                    event_type: ServerEventType::PUT,
                    key: "u1".into(),
                    record: Some(LWWRecord {
                        value: Some(rmpv::Value::String("v1".into())),
                        timestamp: Timestamp {
                            millis: 1,
                            counter: 0,
                            node_id: "n".into(),
                        },
                        ttl_ms: None,
                    }),
                    or_record: None,
                    or_tag: None,
                },
                ServerEventPayload {
                    map_name: "users".into(),
                    event_type: ServerEventType::REMOVE,
                    key: "u2".into(),
                    ..Default::default()
                },
            ],
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ServerBatchEventPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- QueryUpdatePayload ----

    #[test]
    fn query_update_payload_roundtrip() {
        let payload = QueryUpdatePayload {
            query_id: "q-001".into(),
            key: "user-1".into(),
            value: rmpv::Value::String("Alice".into()),
            change_type: ChangeEventType::ENTER,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: QueryUpdatePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn query_update_payload_roundtrip_leave() {
        let payload = QueryUpdatePayload {
            query_id: "q-001".into(),
            key: "user-2".into(),
            value: rmpv::Value::Nil,
            change_type: ChangeEventType::LEAVE,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: QueryUpdatePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- GcPrunePayload ----

    #[test]
    fn gc_prune_payload_roundtrip() {
        let payload = GcPrunePayload {
            older_than: Timestamp {
                millis: 1_699_000_000_000,
                counter: 0,
                node_id: "gc-node".into(),
            },
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: GcPrunePayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- Auth ----

    #[test]
    fn auth_ack_data_roundtrip() {
        let data = AuthAckData {
            protocol_version: Some(1),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: AuthAckData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn auth_ack_data_roundtrip_none() {
        let data = AuthAckData {
            protocol_version: None,
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: AuthAckData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn auth_fail_data_roundtrip() {
        let data = AuthFailData {
            error: Some("invalid token".into()),
            code: Some(401),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: AuthFailData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    #[test]
    fn auth_fail_data_roundtrip_default() {
        let data = AuthFailData::default();
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let decoded: AuthFailData = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(data, decoded);
    }

    // ---- Error ----

    #[test]
    fn error_payload_roundtrip() {
        let payload = ErrorPayload {
            code: 500,
            message: "internal server error".into(),
            details: Some(rmpv::Value::String("stack trace here".into())),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ErrorPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn error_payload_roundtrip_no_details() {
        let payload = ErrorPayload {
            code: 404,
            message: "not found".into(),
            details: None,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: ErrorPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- Lock Events ----

    #[test]
    fn lock_granted_payload_roundtrip() {
        let payload = LockGrantedPayload {
            request_id: "lock-1".into(),
            name: "my-lock".into(),
            fencing_token: 42,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockGrantedPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn lock_released_payload_roundtrip() {
        let payload = LockReleasedPayload {
            request_id: "lock-1".into(),
            name: "my-lock".into(),
            success: true,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockReleasedPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    #[test]
    fn lock_released_payload_roundtrip_failed() {
        let payload = LockReleasedPayload {
            request_id: "lock-2".into(),
            name: "other-lock".into(),
            success: false,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: LockReleasedPayload = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- Sync Reset ----

    #[test]
    fn sync_reset_required_payload_roundtrip() {
        let payload = SyncResetRequiredPayload {
            map_name: "users".into(),
            reason: "partition ownership changed".into(),
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let decoded: SyncResetRequiredPayload =
            rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(payload, decoded);
    }

    // ---- AC-no-type-field: verify no client event struct has a "type" key ----

    #[test]
    fn client_event_structs_have_no_type_key() {
        // Verify representative client event structs do NOT produce a "type" key
        let auth = AuthAckData {
            protocol_version: Some(1),
        };
        let bytes = rmp_serde::to_vec_named(&auth).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");
        let type_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("type"))
            .collect();
        assert!(
            type_keys.is_empty(),
            "AuthAckData should not have a 'type' key"
        );

        let error = ErrorPayload {
            code: 500,
            message: "err".into(),
            details: None,
        };
        let bytes = rmp_serde::to_vec_named(&error).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");
        let type_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("type"))
            .collect();
        assert!(
            type_keys.is_empty(),
            "ErrorPayload should not have a 'type' key"
        );

        // ServerEventPayload has eventType, not "type"
        let se = ServerEventPayload::default();
        let bytes = rmp_serde::to_vec_named(&se).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");
        let type_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("type"))
            .collect();
        assert!(
            type_keys.is_empty(),
            "ServerEventPayload should not have a 'type' key (eventType is camelCase)"
        );
    }

    // ---- AC-flat-vs-wrapped byte inspection ----

    #[test]
    fn flat_data_struct_has_no_payload_key() {
        // Representative flat struct: AuthFailData
        let data = AuthFailData {
            error: Some("err".into()),
            code: Some(403),
        };
        let bytes = rmp_serde::to_vec_named(&data).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");

        let payload_keys: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.as_str() == Some("payload"))
            .collect();
        assert!(
            payload_keys.is_empty(),
            "flat data structs should NOT have a 'payload' key"
        );

        // Verify fields are at top level
        let error_key = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("error"))
            .expect("should have 'error' key at top level");
        assert_eq!(error_key.1.as_str(), Some("err"));
    }

    #[test]
    fn payload_wrapped_struct_serializes_standalone() {
        // ErrorPayload is a payload-wrapped struct. When serialized alone,
        // it produces its own map. The wrapping under "payload" key happens
        // at the Message enum level (SPEC-052e).
        let payload = ErrorPayload {
            code: 400,
            message: "bad request".into(),
            details: None,
        };
        let bytes = rmp_serde::to_vec_named(&payload).expect("serialize");
        let raw: rmpv::Value =
            rmpv::decode::read_value(&mut &bytes[..]).expect("decode as Value");
        let map = raw.as_map().expect("should be map");

        // Verify fields are present directly
        let code_key = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("code"));
        assert!(code_key.is_some(), "should have 'code' key");

        let msg_key = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("message"));
        assert!(msg_key.is_some(), "should have 'message' key");
    }
}
