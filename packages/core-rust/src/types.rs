use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Opaque serialized CRDT record stored in persistence.
/// Placeholder: will be refined when CRDTs are ported (Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageValue {
    /// Raw bytes of the serialized CRDT record.
    pub data: Vec<u8>,
}

/// Generic runtime value type for CRDT map entries.
///
/// Supports all JSON-compatible types plus binary data. Used as the
/// concrete value type in `LWWMap<Value>` and `ORMap<Value>`, and
/// referenced by `SchemaProvider::validate` for schema validation.
///
/// Serializes to `MsgPack` via `rmp-serde` for cross-language compatibility
/// with the TypeScript client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// JSON null.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer (signed 64-bit).
    Int(i64),
    /// JSON floating-point (64-bit IEEE 754).
    Float(f64),
    /// JSON string (UTF-8).
    String(String),
    /// Binary data (not directly representable in JSON).
    Bytes(Vec<u8>),
    /// JSON array (ordered sequence of values).
    Array(Vec<Value>),
    /// JSON object (ordered map of string keys to values).
    /// Uses `BTreeMap` for deterministic serialization order.
    Map(BTreeMap<String, Value>),
}

/// Discriminant for CRDT map types (LWW vs OR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapType {
    /// Last-Write-Wins Map: conflict resolution by highest timestamp.
    Lww,
    /// Observed-Remove Map: supports concurrent additions with unique tags.
    Or,
}

/// Placeholder for the unified CRDT map abstraction.
/// Will be replaced with actual `LWWMap`/`ORMap` implementations in Phase 2.
#[derive(Debug)]
pub struct CrdtMap {
    /// Which CRDT strategy this map uses.
    pub map_type: MapType,
}

/// Authentication principal for multi-tenancy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    /// Unique identifier for the authenticated entity.
    pub id: String,
    /// Roles assigned to this principal for authorization checks.
    pub roles: Vec<String>,
}
