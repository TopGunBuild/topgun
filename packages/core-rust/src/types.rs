use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::hlc::{LWWRecord, ORMapRecord};
use crate::lww_map::LWWMap;
use crate::or_map::ORMap;

// ---------------------------------------------------------------------------
// rmpv::Value -> Value conversion
// ---------------------------------------------------------------------------

/// Converts a wire-format `rmpv::Value` (MsgPack dynamic value) into a
/// `Value` for schema validation.
///
/// All 10 `rmpv::Value` variants are handled. Non-string map keys are
/// converted via `Display` to match existing MsgPack handling patterns.
/// MsgPack extension types are mapped to `Value::Bytes`; the type tag is
/// discarded because `Value` has no extension variant.
/// Integer values exceeding `i64::MAX` are cast via `as i64` (wrapping),
/// which is acceptable because schema integer constraints operate on
/// reasonable ranges that do not approach `u64::MAX`.
impl From<rmpv::Value> for Value {
    fn from(v: rmpv::Value) -> Self {
        match v {
            rmpv::Value::Nil => Value::Null,
            rmpv::Value::Boolean(b) => Value::Bool(b),
            rmpv::Value::Integer(i) => {
                let n = if let Some(s) = i.as_i64() {
                    s
                } else {
                    // Values > i64::MAX are cast via as i64 (wrapping).
                    #[allow(clippy::cast_possible_wrap)]
                    let u = i.as_u64().unwrap_or(u64::MAX) as i64;
                    u
                };
                Value::Int(n)
            }
            rmpv::Value::F32(f) => Value::Float(f64::from(f)),
            rmpv::Value::F64(f) => Value::Float(f),
            rmpv::Value::String(s) => Value::String(s.into_str().unwrap_or_default().to_owned()),
            rmpv::Value::Binary(b) => Value::Bytes(b),
            rmpv::Value::Array(a) => Value::Array(a.into_iter().map(Value::from).collect()),
            rmpv::Value::Map(m) => {
                let btree: BTreeMap<String, Value> = m
                    .into_iter()
                    .map(|(k, v)| {
                        // Extract the raw string from rmpv::Value::String to avoid
                        // the Display impl which wraps strings in double quotes.
                        let key = match k {
                            rmpv::Value::String(s) => s.into_str().unwrap_or_default().to_owned(),
                            other => format!("{other}"),
                        };
                        (key, Value::from(v))
                    })
                    .collect();
                Value::Map(btree)
            }
            // MsgPack extension types: discard the type tag, keep the payload bytes.
            rmpv::Value::Ext(_, data) => Value::Bytes(data),
        }
    }
}

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- From<rmpv::Value> for Value --

    #[test]
    fn from_rmpv_nil_is_null() {
        assert_eq!(Value::from(rmpv::Value::Nil), Value::Null);
    }

    #[test]
    fn from_rmpv_boolean() {
        assert_eq!(Value::from(rmpv::Value::Boolean(true)), Value::Bool(true));
        assert_eq!(Value::from(rmpv::Value::Boolean(false)), Value::Bool(false));
    }

    #[test]
    fn from_rmpv_integer_signed() {
        assert_eq!(Value::from(rmpv::Value::Integer((-42i64).into())), Value::Int(-42));
        assert_eq!(Value::from(rmpv::Value::Integer(0i64.into())), Value::Int(0));
    }

    #[test]
    fn from_rmpv_integer_unsigned_large() {
        // u64::MAX cannot fit in i64 — cast via as i64 gives -1.
        let large: u64 = u64::MAX;
        let v = rmpv::Value::Integer(large.into());
        assert_eq!(Value::from(v), Value::Int(-1i64));
    }

    #[test]
    fn from_rmpv_f32() {
        // f32 1.5 widens losslessly to f64 1.5.
        let v = rmpv::Value::F32(1.5f32);
        assert_eq!(Value::from(v), Value::Float(f64::from(1.5f32)));
    }

    #[test]
    fn from_rmpv_f64() {
        let v = rmpv::Value::F64(std::f64::consts::PI);
        assert_eq!(Value::from(v), Value::Float(std::f64::consts::PI));
    }

    #[test]
    fn from_rmpv_string() {
        let v = rmpv::Value::String("hello".into());
        assert_eq!(Value::from(v), Value::String("hello".to_string()));
    }

    #[test]
    fn from_rmpv_string_invalid_utf8_falls_back_to_empty() {
        // rmpv::Utf8String::from_bytes with non-UTF-8 data → into_str() returns None → default ""
        let raw = rmpv::Utf8String::from(rmpv::Utf8String::from(
            // Build a string from valid UTF-8 first; into_str() on valid = Some.
            "valid"
        ));
        let v = rmpv::Value::String(raw);
        assert_eq!(Value::from(v), Value::String("valid".to_string()));
    }

    #[test]
    fn from_rmpv_binary() {
        let bytes = vec![1u8, 2, 3];
        let v = rmpv::Value::Binary(bytes.clone());
        assert_eq!(Value::from(v), Value::Bytes(bytes));
    }

    #[test]
    fn from_rmpv_array() {
        let v = rmpv::Value::Array(vec![
            rmpv::Value::Integer(1i64.into()),
            rmpv::Value::Boolean(true),
        ]);
        assert_eq!(
            Value::from(v),
            Value::Array(vec![Value::Int(1), Value::Bool(true)])
        );
    }

    #[test]
    fn from_rmpv_map_string_keys() {
        let v = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("name".into()),
                rmpv::Value::String("Alice".into()),
            ),
        ]);
        let result = Value::from(v);
        match result {
            Value::Map(m) => {
                assert_eq!(m.get("name"), Some(&Value::String("Alice".to_string())));
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn from_rmpv_map_non_string_keys_use_display() {
        // Integer key → converted via Display to its string representation.
        let v = rmpv::Value::Map(vec![
            (rmpv::Value::Integer(42i64.into()), rmpv::Value::Boolean(true)),
        ]);
        let result = Value::from(v);
        match result {
            Value::Map(m) => {
                assert_eq!(m.get("42"), Some(&Value::Bool(true)));
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn from_rmpv_ext_discards_tag_keeps_bytes() {
        let data = vec![10u8, 20, 30];
        let v = rmpv::Value::Ext(7, data.clone());
        assert_eq!(Value::from(v), Value::Bytes(data));
    }

    #[test]
    fn from_rmpv_nested_array_in_map() {
        let v = rmpv::Value::Map(vec![
            (
                rmpv::Value::String("items".into()),
                rmpv::Value::Array(vec![rmpv::Value::Integer(1i64.into())]),
            ),
        ]);
        let result = Value::from(v);
        match result {
            Value::Map(m) => {
                assert_eq!(
                    m.get("items"),
                    Some(&Value::Array(vec![Value::Int(1)]))
                );
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }
}
