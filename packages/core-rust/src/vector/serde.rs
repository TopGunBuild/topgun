use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserializer, Serializer};
use std::fmt;

use super::Vector;

/// Serializes `Vector` as a `MsgPack` map with two fields:
/// - `"type"`: string tag (`"f32"`, `"f64"`, `"i32"`, `"i16"`)
/// - `"data"`: binary blob of raw little-endian bytes
///
/// Uses `serde_bytes` semantics for the data field so `MsgPack` encodes it
/// as a binary blob (ext/bin), not a byte array.
impl serde::Serialize for Vector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        match self {
            Vector::F32(v) => {
                map.serialize_entry("type", "f32")?;
                let bytes = floats_to_bytes_f32(v);
                map.serialize_entry("data", serde_bytes::Bytes::new(&bytes))?;
            }
            Vector::F64(v) => {
                map.serialize_entry("type", "f64")?;
                let bytes = floats_to_bytes_f64(v);
                map.serialize_entry("data", serde_bytes::Bytes::new(&bytes))?;
            }
            Vector::I32(v) => {
                map.serialize_entry("type", "i32")?;
                let bytes = ints_to_bytes_i32(v);
                map.serialize_entry("data", serde_bytes::Bytes::new(&bytes))?;
            }
            Vector::I16(v) => {
                map.serialize_entry("type", "i16")?;
                let bytes = ints_to_bytes_i16(v);
                map.serialize_entry("data", serde_bytes::Bytes::new(&bytes))?;
            }
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for Vector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(VectorVisitor)
    }
}

struct VectorVisitor;

impl<'de> Visitor<'de> for VectorVisitor {
    type Value = Vector;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a map with 'type' and 'data' fields")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut type_tag: Option<String> = None;
        let mut data: Option<serde_bytes::ByteBuf> = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "type" => {
                    type_tag = Some(map.next_value()?);
                }
                "data" => {
                    data = Some(map.next_value()?);
                }
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }

        let type_tag = type_tag.ok_or_else(|| de::Error::missing_field("type"))?;
        let data = data.ok_or_else(|| de::Error::missing_field("data"))?;
        let bytes = data.into_vec();

        match type_tag.as_str() {
            "f32" => Ok(Vector::F32(bytes_to_f32(&bytes).map_err(de::Error::custom)?)),
            "f64" => Ok(Vector::F64(bytes_to_f64(&bytes).map_err(de::Error::custom)?)),
            "i32" => Ok(Vector::I32(bytes_to_i32(&bytes).map_err(de::Error::custom)?)),
            "i16" => Ok(Vector::I16(bytes_to_i16(&bytes).map_err(de::Error::custom)?)),
            other => Err(de::Error::unknown_variant(other, &["f32", "f64", "i32", "i16"])),
        }
    }
}

// --- Byte conversion helpers (little-endian) ---

fn floats_to_bytes_f32(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn floats_to_bytes_f64(v: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 8);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn ints_to_bytes_i32(v: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn ints_to_bytes_i16(v: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 2);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn bytes_to_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("f32 data length {} is not a multiple of 4", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

fn bytes_to_f64(bytes: &[u8]) -> Result<Vec<f64>, String> {
    if !bytes.len().is_multiple_of(8) {
        return Err(format!("f64 data length {} is not a multiple of 8", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

fn bytes_to_i32(bytes: &[u8]) -> Result<Vec<i32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("i32 data length {} is not a multiple of 4", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

fn bytes_to_i16(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(format!("i16 data length {} is not a multiple of 2", bytes.len()));
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes(c.try_into().unwrap()))
        .collect())
}
