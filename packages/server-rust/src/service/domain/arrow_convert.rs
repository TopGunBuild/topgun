//! MsgPack-to-Arrow conversion utilities.
//!
//! Converts `rmpv::Value` entries from `RecordStore` iteration into Arrow
//! `RecordBatch` instances for DataFusion query execution.
//!
//! All types in this module are feature-gated behind `#[cfg(feature = "datafusion")]`.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, ListBuilder,
    RecordBatch, StringBuilder, TimestampMillisecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use topgun_core::MapSchema;

// ---------------------------------------------------------------------------
// Schema helpers
// ---------------------------------------------------------------------------

/// Builds an Arrow `Schema` from a `MapSchema` with a prepended `_key` column.
///
/// The `_key` column is always the first field (Utf8, non-nullable) and
/// represents the record key from the `RecordStore`. Remaining fields come
/// from `MapSchema::to_arrow_schema()`.
#[must_use]
pub fn make_arrow_schema(map_schema: &MapSchema) -> Schema {
    let inner = map_schema.to_arrow_schema();
    let mut fields = vec![Field::new("_key", DataType::Utf8, false)];
    fields.extend(inner.fields().iter().map(|f| f.as_ref().clone()));
    Schema::new(fields)
}

// ---------------------------------------------------------------------------
// RecordBatch builder
// ---------------------------------------------------------------------------

/// Converts key-value entries into an Arrow `RecordBatch`.
///
/// Each entry is a `(key, rmpv::Value)` pair from `RecordStore` iteration.
/// The `schema` must have `_key` as the first field. Entries whose value is
/// not `rmpv::Value::Map` are skipped with a warning.
///
/// Per-column builders are created based on the Arrow schema field types,
/// and values are extracted from the `rmpv::Value::Map` by field name.
pub fn build_record_batch(
    entries: &[(String, rmpv::Value)],
    schema: &Schema,
) -> Result<RecordBatch, anyhow::Error> {
    let fields = schema.fields();
    let num_fields = fields.len();

    // Create per-column builders.
    let mut builders: Vec<ColumnBuilder> = fields
        .iter()
        .map(|f| ColumnBuilder::new(f.data_type()))
        .collect();

    for (key, value) in entries {
        let map_entries = match value {
            rmpv::Value::Map(entries) => entries,
            _ => {
                tracing::warn!(
                    key = key.as_str(),
                    "skipping entry: LWW value is not a Map, cannot extract fields"
                );
                continue;
            }
        };

        // First column is always _key.
        builders[0].append_string(key);

        // Remaining columns from the map.
        for (col_idx, field) in fields.iter().enumerate().skip(1) {
            let field_value = find_field_value(map_entries, field.name());
            builders[col_idx].append_rmpv(field_value, field.data_type());
        }
    }

    // Finalize builders into arrays.
    let columns: Vec<ArrayRef> = builders.into_iter().map(|b| b.finish()).collect();

    if columns.is_empty() || (num_fields > 0 && columns[0].len() == 0 && entries.is_empty()) {
        // Empty batch with correct schema.
        return Ok(RecordBatch::new_empty(Arc::new(schema.clone())));
    }

    Ok(RecordBatch::try_new(Arc::new(schema.clone()), columns)?)
}

/// Finds a field value in an `rmpv::Value::Map` by string key.
fn find_field_value<'a>(
    map_entries: &'a [(rmpv::Value, rmpv::Value)],
    field_name: &str,
) -> Option<&'a rmpv::Value> {
    for (k, v) in map_entries {
        if let rmpv::Value::String(s) = k {
            if s.as_str() == Some(field_name) {
                return Some(v);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// ColumnBuilder -- per-type array builder wrapper
// ---------------------------------------------------------------------------

/// Wrapper around typed Arrow array builders to handle dynamic dispatch.
enum ColumnBuilder {
    Utf8(StringBuilder),
    Int64(Int64Builder),
    Float64(Float64Builder),
    Boolean(BooleanBuilder),
    Binary(BinaryBuilder),
    TimestampMs(TimestampMillisecondBuilder),
    ListInt64(ListBuilder<Int64Builder>),
    ListFloat64(ListBuilder<Float64Builder>),
    ListUtf8(ListBuilder<StringBuilder>),
    ListBool(ListBuilder<BooleanBuilder>),
    /// Fallback: serialize to JSON string.
    JsonFallback(StringBuilder),
}

impl ColumnBuilder {
    fn new(data_type: &DataType) -> Self {
        match data_type {
            DataType::Utf8 => Self::Utf8(StringBuilder::new()),
            DataType::Int64 => Self::Int64(Int64Builder::new()),
            DataType::Float64 => Self::Float64(Float64Builder::new()),
            DataType::Boolean => Self::Boolean(BooleanBuilder::new()),
            DataType::Binary => Self::Binary(BinaryBuilder::new()),
            DataType::Timestamp(TimeUnit::Millisecond, _) => {
                Self::TimestampMs(TimestampMillisecondBuilder::new())
            }
            DataType::List(inner_field) => match inner_field.data_type() {
                DataType::Int64 => Self::ListInt64(ListBuilder::new(Int64Builder::new())),
                DataType::Float64 => Self::ListFloat64(ListBuilder::new(Float64Builder::new())),
                DataType::Boolean => Self::ListBool(ListBuilder::new(BooleanBuilder::new())),
                DataType::Utf8 | _ => Self::ListUtf8(ListBuilder::new(StringBuilder::new())),
            },
            _ => Self::JsonFallback(StringBuilder::new()),
        }
    }

    /// Append a string value (used for the `_key` column).
    fn append_string(&mut self, s: &str) {
        match self {
            Self::Utf8(b) => b.append_value(s),
            _ => {
                // _key is always Utf8, but handle defensively.
                if let Self::JsonFallback(b) = self {
                    b.append_value(s);
                }
            }
        }
    }

    /// Append an `rmpv::Value` based on the target Arrow data type.
    fn append_rmpv(&mut self, value: Option<&rmpv::Value>, _data_type: &DataType) {
        match value {
            None | Some(rmpv::Value::Nil) => self.append_null(),
            Some(v) => self.append_value(v),
        }
    }

    fn append_null(&mut self) {
        match self {
            Self::Utf8(b) => b.append_null(),
            Self::Int64(b) => b.append_null(),
            Self::Float64(b) => b.append_null(),
            Self::Boolean(b) => b.append_null(),
            Self::Binary(b) => b.append_null(),
            Self::TimestampMs(b) => b.append_null(),
            Self::ListInt64(b) => b.append_null(),
            Self::ListFloat64(b) => b.append_null(),
            Self::ListUtf8(b) => b.append_null(),
            Self::ListBool(b) => b.append_null(),
            Self::JsonFallback(b) => b.append_null(),
        }
    }

    fn append_value(&mut self, value: &rmpv::Value) {
        match self {
            Self::Utf8(b) => match value {
                rmpv::Value::String(s) => {
                    b.append_value(s.as_str().unwrap_or(""));
                }
                _ => {
                    // Serialize non-string to JSON as fallback.
                    b.append_value(rmpv_to_json_string(value));
                }
            },
            Self::Int64(b) => match value {
                rmpv::Value::Integer(i) => {
                    b.append_value(i.as_i64().unwrap_or(0));
                }
                _ => b.append_null(),
            },
            Self::Float64(b) => match value {
                rmpv::Value::F64(f) => b.append_value(*f),
                rmpv::Value::F32(f) => b.append_value(f64::from(*f)),
                rmpv::Value::Integer(i) => {
                    b.append_value(i.as_f64().unwrap_or(0.0));
                }
                _ => b.append_null(),
            },
            Self::Boolean(b) => match value {
                rmpv::Value::Boolean(v) => b.append_value(*v),
                _ => b.append_null(),
            },
            Self::Binary(b) => match value {
                rmpv::Value::Binary(v) => b.append_value(v),
                _ => b.append_null(),
            },
            Self::TimestampMs(b) => match value {
                rmpv::Value::Integer(i) => {
                    b.append_value(i.as_i64().unwrap_or(0));
                }
                _ => b.append_null(),
            },
            Self::ListInt64(b) => {
                if let rmpv::Value::Array(arr) = value {
                    b.values().append_slice(
                        &arr.iter()
                            .map(|v| match v {
                                rmpv::Value::Integer(i) => i.as_i64().unwrap_or(0),
                                _ => 0,
                            })
                            .collect::<Vec<_>>(),
                    );
                    b.append(true);
                } else {
                    b.append_null();
                }
            }
            Self::ListFloat64(b) => {
                if let rmpv::Value::Array(arr) = value {
                    b.values().append_slice(
                        &arr.iter()
                            .map(|v| match v {
                                rmpv::Value::F64(f) => *f,
                                rmpv::Value::Integer(i) => i.as_f64().unwrap_or(0.0),
                                _ => 0.0,
                            })
                            .collect::<Vec<_>>(),
                    );
                    b.append(true);
                } else {
                    b.append_null();
                }
            }
            Self::ListUtf8(b) => {
                if let rmpv::Value::Array(arr) = value {
                    for item in arr {
                        match item {
                            rmpv::Value::String(s) => {
                                b.values().append_value(s.as_str().unwrap_or(""));
                            }
                            _ => {
                                b.values().append_value(rmpv_to_json_string(item));
                            }
                        }
                    }
                    b.append(true);
                } else {
                    b.append_null();
                }
            }
            Self::ListBool(b) => {
                if let rmpv::Value::Array(arr) = value {
                    for item in arr {
                        match item {
                            rmpv::Value::Boolean(v) => b.values().append_value(*v),
                            _ => b.values().append_null(),
                        }
                    }
                    b.append(true);
                } else {
                    b.append_null();
                }
            }
            Self::JsonFallback(b) => {
                b.append_value(rmpv_to_json_string(value));
            }
        }
    }

    fn finish(self) -> ArrayRef {
        match self {
            Self::Utf8(mut b) => Arc::new(b.finish()),
            Self::Int64(mut b) => Arc::new(b.finish()),
            Self::Float64(mut b) => Arc::new(b.finish()),
            Self::Boolean(mut b) => Arc::new(b.finish()),
            Self::Binary(mut b) => Arc::new(b.finish()),
            Self::TimestampMs(mut b) => Arc::new(b.finish()),
            Self::ListInt64(mut b) => Arc::new(b.finish()),
            Self::ListFloat64(mut b) => Arc::new(b.finish()),
            Self::ListUtf8(mut b) => Arc::new(b.finish()),
            Self::ListBool(mut b) => Arc::new(b.finish()),
            Self::JsonFallback(mut b) => Arc::new(b.finish()),
        }
    }
}

/// Converts an `rmpv::Value` to a JSON string for fallback serialization.
fn rmpv_to_json_string(value: &rmpv::Value) -> String {
    match value {
        rmpv::Value::Nil => "null".to_string(),
        rmpv::Value::Boolean(b) => b.to_string(),
        rmpv::Value::Integer(i) => {
            if let Some(v) = i.as_i64() {
                v.to_string()
            } else if let Some(v) = i.as_u64() {
                v.to_string()
            } else {
                "0".to_string()
            }
        }
        rmpv::Value::F32(f) => f.to_string(),
        rmpv::Value::F64(f) => f.to_string(),
        rmpv::Value::String(s) => {
            format!("\"{}\"", s.as_str().unwrap_or(""))
        }
        rmpv::Value::Binary(b) => {
            format!("{b:?}")
        }
        rmpv::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(rmpv_to_json_string).collect();
            format!("[{}]", items.join(","))
        }
        rmpv::Value::Map(entries) => {
            let items: Vec<String> = entries
                .iter()
                .map(|(k, v)| {
                    let key_str = match k {
                        rmpv::Value::String(s) => s.as_str().unwrap_or("").to_string(),
                        _ => rmpv_to_json_string(k),
                    };
                    format!("\"{}\":{}", key_str, rmpv_to_json_string(v))
                })
                .collect();
            format!("{{{}}}", items.join(","))
        }
        rmpv::Value::Ext(_, _) => "null".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use arrow::array::Array;
    use super::*;
    use topgun_core::{FieldDef, FieldType};

    fn make_map_schema(fields: Vec<(&str, FieldType, bool)>) -> MapSchema {
        MapSchema {
            version: 1,
            strict: false,
            fields: fields
                .into_iter()
                .map(|(name, ft, required)| FieldDef {
                    name: name.to_string(),
                    required,
                    field_type: ft,
                    constraints: None,
                })
                .collect(),
        }
    }

    fn make_rmpv_map(fields: Vec<(&str, rmpv::Value)>) -> rmpv::Value {
        rmpv::Value::Map(
            fields
                .into_iter()
                .map(|(k, v)| (rmpv::Value::String(k.into()), v))
                .collect(),
        )
    }

    #[test]
    fn make_arrow_schema_prepends_key_column() {
        let ms = make_map_schema(vec![
            ("name", FieldType::String, true),
            ("age", FieldType::Int, false),
        ]);
        let schema = make_arrow_schema(&ms);

        assert_eq!(schema.fields().len(), 3);
        assert_eq!(schema.field(0).name(), "_key");
        assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
        assert!(!schema.field(0).is_nullable());
        assert_eq!(schema.field(1).name(), "name");
        assert_eq!(schema.field(2).name(), "age");
    }

    #[test]
    fn build_record_batch_basic() {
        let ms = make_map_schema(vec![
            ("name", FieldType::String, true),
            ("age", FieldType::Int, false),
        ]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![
            (
                "user-1".to_string(),
                make_rmpv_map(vec![
                    ("name", rmpv::Value::String("Alice".into())),
                    ("age", rmpv::Value::Integer(30.into())),
                ]),
            ),
            (
                "user-2".to_string(),
                make_rmpv_map(vec![
                    ("name", rmpv::Value::String("Bob".into())),
                    ("age", rmpv::Value::Integer(25.into())),
                ]),
            ),
        ];

        let batch = build_record_batch(&entries, &schema).expect("build should succeed");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);

        // Check _key column.
        let keys = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();
        assert_eq!(keys.value(0), "user-1");
        assert_eq!(keys.value(1), "user-2");

        // Check name column.
        let names = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "Alice");
        assert_eq!(names.value(1), "Bob");

        // Check age column.
        let ages = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert_eq!(ages.value(0), 30);
        assert_eq!(ages.value(1), 25);
    }

    #[test]
    fn build_record_batch_handles_null_values() {
        let ms = make_map_schema(vec![("score", FieldType::Float, false)]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![("score", rmpv::Value::Nil)]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        assert_eq!(batch.num_rows(), 1);
        let scores = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::Float64Array>()
            .unwrap();
        assert!(scores.is_null(0));
    }

    #[test]
    fn build_record_batch_skips_non_map_values() {
        let ms = make_map_schema(vec![("name", FieldType::String, false)]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![
            ("k1".to_string(), rmpv::Value::String("not a map".into())),
            (
                "k2".to_string(),
                make_rmpv_map(vec![("name", rmpv::Value::String("valid".into()))]),
            ),
        ];

        let batch = build_record_batch(&entries, &schema).unwrap();
        // Only the valid entry should be in the batch.
        assert_eq!(batch.num_rows(), 1);
    }

    #[test]
    fn build_record_batch_boolean_column() {
        let ms = make_map_schema(vec![("active", FieldType::Bool, false)]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![("active", rmpv::Value::Boolean(true))]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        let bools = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::BooleanArray>()
            .unwrap();
        assert!(bools.value(0));
    }

    #[test]
    fn build_record_batch_binary_column() {
        let ms = make_map_schema(vec![("data", FieldType::Binary, false)]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![("data", rmpv::Value::Binary(vec![1, 2, 3]))]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        let bins = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::BinaryArray>()
            .unwrap();
        assert_eq!(bins.value(0), &[1, 2, 3]);
    }

    #[test]
    fn build_record_batch_timestamp_column() {
        let ms = make_map_schema(vec![("created_at", FieldType::Timestamp, false)]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![
                ("created_at", rmpv::Value::Integer(1_700_000_000_000i64.into())),
            ]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        let ts = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::TimestampMillisecondArray>()
            .unwrap();
        assert_eq!(ts.value(0), 1_700_000_000_000);
    }

    #[test]
    fn build_record_batch_list_column() {
        let ms = make_map_schema(vec![(
            "tags",
            FieldType::Array(Box::new(FieldType::String)),
            false,
        )]);
        let schema = make_arrow_schema(&ms);

        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![(
                "tags",
                rmpv::Value::Array(vec![
                    rmpv::Value::String("rust".into()),
                    rmpv::Value::String("arrow".into()),
                ]),
            )]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        assert_eq!(batch.num_rows(), 1);
        // The list column should have 1 row with 2 elements.
        let list = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::ListArray>()
            .unwrap();
        assert_eq!(list.len(), 1);
        let inner = list.value(0);
        assert_eq!(inner.len(), 2);
    }

    #[test]
    fn build_record_batch_map_value_serialized_as_json() {
        let ms = make_map_schema(vec![("meta", FieldType::Map, false)]);
        let schema = make_arrow_schema(&ms);

        let nested_map = rmpv::Value::Map(vec![(
            rmpv::Value::String("nested".into()),
            rmpv::Value::Integer(42.into()),
        )]);
        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![("meta", nested_map)]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        let strs = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .unwrap();
        let json_str = strs.value(0);
        assert!(json_str.contains("nested"));
        assert!(json_str.contains("42"));
    }

    #[test]
    fn build_record_batch_missing_field_becomes_null() {
        let ms = make_map_schema(vec![
            ("name", FieldType::String, false),
            ("age", FieldType::Int, false),
        ]);
        let schema = make_arrow_schema(&ms);

        // Entry only has "name", missing "age".
        let entries = vec![(
            "k1".to_string(),
            make_rmpv_map(vec![("name", rmpv::Value::String("Alice".into()))]),
        )];

        let batch = build_record_batch(&entries, &schema).unwrap();
        assert_eq!(batch.num_rows(), 1);
        let ages = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        assert!(ages.is_null(0));
    }

    #[test]
    fn build_record_batch_empty_entries() {
        let ms = make_map_schema(vec![("name", FieldType::String, false)]);
        let schema = make_arrow_schema(&ms);

        let batch = build_record_batch(&[], &schema).unwrap();
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 2); // _key + name
    }
}
