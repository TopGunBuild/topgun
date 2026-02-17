//! Wire-compatible message schemas for the `TopGun` protocol.
//!
//! Each submodule corresponds to a domain of messages defined in the TypeScript
//! Zod schemas (`packages/core/src/schemas/`). All types use named `MsgPack`
//! serialization (`rmp_serde::to_vec_named()`) with camelCase field names to
//! match the TypeScript wire format.

pub mod base;

pub mod query;
pub mod sync;

// Future submodules (SPEC-052c through SPEC-052e):
// pub mod search;
// pub mod cluster;
// pub mod messaging;
// pub mod client_events;
// pub mod http_sync;

pub use base::{
    AuthMessage, AuthRequiredMessage, ChangeEventType, ClientOp, PredicateNode, PredicateOp,
    Query, SortDirection, WriteConcern,
};

pub use query::{
    CursorStatus, QueryRespMessage, QueryRespPayload, QueryResultEntry, QuerySubMessage,
    QuerySubPayload, QueryUnsubMessage, QueryUnsubPayload,
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
