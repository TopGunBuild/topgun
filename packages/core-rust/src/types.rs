use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::hlc::{LWWRecord, ORMapRecord};
use crate::lww_map::LWWMap;
use crate::or_map::ORMap;

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

/// Unified CRDT map abstraction wrapping both LWW and OR map types.
///
/// Provides a single type for downstream consumers that need to work with
/// either [`LWWMap<Value>`] or [`ORMap<Value>`] without knowing which
/// strategy is in use.
///
/// # Note
///
/// `Debug` is implemented manually because `LWWMap` and `ORMap` contain
/// [`HLC`](crate::HLC) which holds a `Box<dyn ClockSource>` that cannot
/// auto-derive `Debug`.
pub enum CrdtMap {
    /// Last-Write-Wins Map: conflict resolution by highest timestamp.
    Lww(LWWMap<Value>),
    /// Observed-Remove Map: supports concurrent additions with unique tags.
    Or(ORMap<Value>),
}

impl CrdtMap {
    /// Returns which CRDT strategy this map uses.
    #[must_use]
    pub fn map_type(&self) -> MapType {
        match self {
            Self::Lww(_) => MapType::Lww,
            Self::Or(_) => MapType::Or,
        }
    }
}

impl fmt::Debug for CrdtMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lww(_) => f.debug_tuple("CrdtMap::Lww").field(&"LWWMap<Value>").finish(),
            Self::Or(_) => f.debug_tuple("CrdtMap::Or").field(&"ORMap<Value>").finish(),
        }
    }
}

impl StorageValue {
    /// Creates a `StorageValue` from an [`LWWRecord<V>`] by serializing it to `MsgPack`.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn from_lww_record<V: Serialize>(record: &LWWRecord<V>) -> Result<Self, rmp_serde::encode::Error> {
        let data = rmp_serde::to_vec(record)?;
        Ok(Self { data })
    }

    /// Creates a `StorageValue` from an [`ORMapRecord<V>`] by serializing it to `MsgPack`.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn from_or_map_record<V: Serialize>(record: &ORMapRecord<V>) -> Result<Self, rmp_serde::encode::Error> {
        let data = rmp_serde::to_vec(record)?;
        Ok(Self { data })
    }
}

/// Authentication principal for multi-tenancy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    /// Unique identifier for the authenticated entity.
    pub id: String,
    /// Roles assigned to this principal for authorization checks.
    pub roles: Vec<String>,
}
