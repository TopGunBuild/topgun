use serde::{Deserialize, Serialize};

/// Opaque serialized CRDT record stored in persistence.
/// Placeholder: will be refined when CRDTs are ported (Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageValue {
    /// Raw bytes of the serialized CRDT record.
    pub data: Vec<u8>,
}

/// Generic runtime value type for CRDT map entries.
/// Placeholder: will become a proper enum (Null, Bool, Int, Float, String, Bytes, Array, Map)
/// when message schemas are ported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Value {
    /// Raw bytes of the serialized value.
    pub data: Vec<u8>,
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
