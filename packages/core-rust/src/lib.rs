//! `TopGun` Core -- CRDTs, Hybrid Logical Clock, `MerkleTree`, and message schemas.
//!
//! This crate provides the foundation layer for the `TopGun` data grid:
//!
//! - **HLC** ([`hlc`]): Hybrid Logical Clock for distributed causality tracking
//! - **Hash** ([`hash`]): FNV-1a hash utilities for `MerkleTree` bucket routing
//! - **`MerkleTree`** ([`merkle`]): Prefix trie for efficient delta synchronization
//! - **`LWWMap`** ([`lww_map`]): Last-Write-Wins Map CRDT with `MerkleTree` integration
//! - **`ORMap`** ([`or_map`]): Observed-Remove Map CRDT with add-wins semantics
//! - **Types** ([`types`]): `Value` enum, `StorageValue`, `MapType`, `CrdtMap`, `Principal`
//! - **Traits** ([`traits`]): `Processor`, `QueryNotifier` for DAG execution and live queries
//! - **Schema** ([`schema`]): `MapSchema`, `SyncShape`, `Predicate` for validation and shapes
//! - **Context** ([`context`]): `RequestContext` for per-request identity and tracing

pub mod context;
pub mod hash;
pub mod hlc;
pub mod lww_map;
pub mod merkle;
pub mod or_map;
pub mod schema;
pub mod traits;
pub mod types;

// Context
pub use context::RequestContext;

// Schema
pub use schema::{FieldDef, MapSchema, Predicate, SyncShape, ValidationResult};

// Traits
pub use traits::{Inbox, Processor, ProcessorContext, QueryNotifier};

// Types
pub use types::{CrdtMap, MapType, Principal, StorageValue, Value};

// HLC
pub use hlc::{ClockSource, SystemClock, Timestamp, HLC, LWWRecord, MergeKeyResult, ORMapRecord};

// Hash
pub use hash::{combine_hashes, fnv1a_hash};

// Merkle
pub use merkle::{MerkleNode, MerkleTree, ORMapMerkleTree};

// LWWMap
pub use lww_map::LWWMap;

// ORMap
pub use or_map::ORMap;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn crate_loads() {
        // Empty body: if this test runs, the crate compiles and loads.
    }

    #[test]
    fn timestamp_msgpack_roundtrip() {
        let ts = Timestamp {
            millis: 1_700_000_000_000,
            counter: 42,
            node_id: "node-abc-123".to_string(),
        };
        let bytes = rmp_serde::to_vec(&ts).expect("serialize Timestamp");
        let decoded: Timestamp = rmp_serde::from_slice(&bytes).expect("deserialize Timestamp");
        assert_eq!(ts, decoded);
    }

    #[test]
    fn value_null_msgpack_roundtrip() {
        let val = Value::Null;
        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_bool_msgpack_roundtrip() {
        for b in [true, false] {
            let val = Value::Bool(b);
            let bytes = rmp_serde::to_vec(&val).expect("serialize");
            let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(val, decoded);
        }
    }

    #[test]
    fn value_int_msgpack_roundtrip() {
        for i in [0_i64, -1, 1, i64::MIN, i64::MAX] {
            let val = Value::Int(i);
            let bytes = rmp_serde::to_vec(&val).expect("serialize");
            let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(val, decoded);
        }
    }

    #[test]
    fn value_float_msgpack_roundtrip() {
        let val = Value::Float(3.14);
        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_string_msgpack_roundtrip() {
        let val = Value::String("hello world".to_string());
        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_bytes_msgpack_roundtrip() {
        let val = Value::Bytes(vec![0, 1, 2, 255]);
        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_array_msgpack_roundtrip() {
        let val = Value::Array(vec![
            Value::Null,
            Value::Bool(true),
            Value::Int(42),
            Value::String("nested".to_string()),
        ]);
        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_map_msgpack_roundtrip() {
        let mut map = BTreeMap::new();
        map.insert("name".to_string(), Value::String("Alice".to_string()));
        map.insert("age".to_string(), Value::Int(30));
        map.insert("active".to_string(), Value::Bool(true));

        let val = Value::Map(map);
        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_nested_complex_msgpack_roundtrip() {
        let mut inner_map = BTreeMap::new();
        inner_map.insert("x".to_string(), Value::Float(1.5));
        inner_map.insert("y".to_string(), Value::Float(2.5));

        let val = Value::Map({
            let mut m = BTreeMap::new();
            m.insert("coords".to_string(), Value::Map(inner_map));
            m.insert(
                "tags".to_string(),
                Value::Array(vec![
                    Value::String("a".to_string()),
                    Value::String("b".to_string()),
                ]),
            );
            m.insert("data".to_string(), Value::Bytes(vec![0xDE, 0xAD]));
            m
        });

        let bytes = rmp_serde::to_vec(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    /// Verify all re-exports are accessible from the crate root.
    #[test]
    fn reexports_accessible() {
        // HLC types
        let _ts = Timestamp { millis: 0, counter: 0, node_id: String::new() };
        let _ = SystemClock;

        // Hash functions
        let _ = fnv1a_hash("test");
        let _ = combine_hashes(&[1, 2, 3]);

        // Merkle types
        let _tree = MerkleTree::new(3);
        let _or_tree = ORMapMerkleTree::new(3);

        // LWWMap
        let hlc = HLC::new("test".to_string(), Box::new(SystemClock));
        let _lww: LWWMap<Value> = LWWMap::new(hlc);

        // ORMap
        let hlc2 = HLC::new("test2".to_string(), Box::new(SystemClock));
        let _or: ORMap<Value> = ORMap::new(hlc2);

        // Value enum
        let _ = Value::Null;

        // Other re-exports
        let _ = MapType::Lww;
        let _ = MapType::Or;

        // CrdtMap
        let hlc3 = HLC::new("test3".to_string(), Box::new(SystemClock));
        let lww_map: LWWMap<Value> = LWWMap::new(hlc3);
        let crdt = CrdtMap::Lww(lww_map);
        assert_eq!(crdt.map_type(), MapType::Lww);

        let hlc4 = HLC::new("test4".to_string(), Box::new(SystemClock));
        let or_map: ORMap<Value> = ORMap::new(hlc4);
        let crdt_or = CrdtMap::Or(or_map);
        assert_eq!(crdt_or.map_type(), MapType::Or);
    }

    /// `ORMapRecord<Value>` round-trips through MsgPack without data loss (AC-5).
    #[test]
    fn or_map_record_msgpack_roundtrip() {
        let record = ORMapRecord {
            value: Value::String("hello".to_string()),
            timestamp: Timestamp {
                millis: 1_700_000_000_000,
                counter: 42,
                node_id: "node-abc-123".to_string(),
            },
            tag: "1700000000000:42:node-abc-123".to_string(),
            ttl_ms: Some(5000),
        };
        let bytes = rmp_serde::to_vec(&record).expect("serialize ORMapRecord<Value>");
        let decoded: ORMapRecord<Value> =
            rmp_serde::from_slice(&bytes).expect("deserialize ORMapRecord<Value>");
        assert_eq!(record, decoded);
    }

    /// `ORMapRecord<Value>` round-trip with all Value variants.
    #[test]
    fn or_map_record_all_variants_roundtrip() {
        let variants: Vec<ORMapRecord<Value>> = vec![
            ORMapRecord {
                value: Value::Null,
                timestamp: Timestamp { millis: 1, counter: 0, node_id: "n".to_string() },
                tag: "1:0:n".to_string(),
                ttl_ms: None,
            },
            ORMapRecord {
                value: Value::Bool(true),
                timestamp: Timestamp { millis: 2, counter: 0, node_id: "n".to_string() },
                tag: "2:0:n".to_string(),
                ttl_ms: None,
            },
            ORMapRecord {
                value: Value::Int(-42),
                timestamp: Timestamp { millis: 3, counter: 0, node_id: "n".to_string() },
                tag: "3:0:n".to_string(),
                ttl_ms: Some(1000),
            },
            ORMapRecord {
                value: Value::Float(3.14),
                timestamp: Timestamp { millis: 4, counter: 0, node_id: "n".to_string() },
                tag: "4:0:n".to_string(),
                ttl_ms: None,
            },
            ORMapRecord {
                value: Value::Bytes(vec![0xDE, 0xAD]),
                timestamp: Timestamp { millis: 5, counter: 0, node_id: "n".to_string() },
                tag: "5:0:n".to_string(),
                ttl_ms: None,
            },
            ORMapRecord {
                value: Value::Map({
                    let mut m = BTreeMap::new();
                    m.insert("key".to_string(), Value::String("val".to_string()));
                    m
                }),
                timestamp: Timestamp { millis: 6, counter: 0, node_id: "n".to_string() },
                tag: "6:0:n".to_string(),
                ttl_ms: None,
            },
        ];

        for record in variants {
            let bytes = rmp_serde::to_vec(&record).expect("serialize");
            let decoded: ORMapRecord<Value> = rmp_serde::from_slice(&bytes).expect("deserialize");
            assert_eq!(record, decoded);
        }
    }

    /// `StorageValue::from_lww_record` and `from_or_map_record` produce valid data.
    #[test]
    fn storage_value_from_record_conversions() {
        let lww_record = LWWRecord {
            value: Some(Value::String("test".to_string())),
            timestamp: Timestamp { millis: 100, counter: 0, node_id: "n".to_string() },
            ttl_ms: None,
        };
        let sv = StorageValue::from_lww_record(&lww_record).expect("from_lww_record");
        assert!(!sv.data.is_empty());

        let or_record = ORMapRecord {
            value: Value::Int(42),
            timestamp: Timestamp { millis: 200, counter: 0, node_id: "n".to_string() },
            tag: "200:0:n".to_string(),
            ttl_ms: None,
        };
        let sv2 = StorageValue::from_or_map_record(&or_record).expect("from_or_map_record");
        assert!(!sv2.data.is_empty());
    }
}
