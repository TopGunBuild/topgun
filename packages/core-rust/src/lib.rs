//! `TopGun` Core -- CRDTs, Hybrid Logical Clock, `MerkleTree`, and message schemas.
//!
//! This crate provides the foundation layer for the `TopGun` data grid:
//!
//! - **HLC** ([`hlc`]): Hybrid Logical Clock for distributed causality tracking
//! - **Hash** ([`hash`]): FNV-1a hash utilities for `MerkleTree` bucket routing
//! - **`MerkleTree`** ([`merkle`]): Prefix trie for efficient delta synchronization
//! - **Types** ([`types`]): `Value` enum, `StorageValue`, `MapType`, `CrdtMap`, `Principal`
//! - **Traits** ([`traits`]): `Processor`, `QueryNotifier` for DAG execution and live queries
//! - **Schema** ([`schema`]): `MapSchema`, `SyncShape`, `Predicate` for validation and shapes
//! - **Context** ([`context`]): `RequestContext` for per-request identity and tracing

pub mod context;
pub mod hash;
pub mod hlc;
pub mod merkle;
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

        // Value enum
        let _ = Value::Null;

        // Other re-exports
        let _ = MapType::Lww;
    }
}
