use serde::{Deserialize, Serialize};

use crate::messages::base::PredicateNode;
use crate::types::Value;

// ---------------------------------------------------------------------------
// FieldType
// ---------------------------------------------------------------------------

/// Type of a field in a `MapSchema`.
///
/// Controls which `Value` variants are accepted during validation. `Any` is
/// the default to preserve backward compatibility with pre-schema data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum FieldType {
    /// Matches `Value::String`.
    String,
    /// Matches `Value::Int`.
    Int,
    /// Matches `Value::Float` or `Value::Int` (int-to-float widening coercion).
    /// JS clients often send integers where floats are expected.
    Float,
    /// Matches `Value::Bool`.
    Bool,
    /// Matches `Value::Bytes`.
    Binary,
    /// Matches `Value::Int` (epoch millis, i64).
    Timestamp,
    /// Matches `Value::Array`; each element is checked against the inner type.
    Array(Box<FieldType>),
    /// Matches `Value::Map` (nested map; content is not recursively validated in v1).
    Map,
    /// Matches any non-Null `Value` variant. Default for backward compatibility.
    #[default]
    Any,
}

// ---------------------------------------------------------------------------
// FieldConstraint
// ---------------------------------------------------------------------------

/// Optional constraints that further restrict a field's allowed values.
///
/// All fields are optional — omit any constraint that is not needed.
/// Applied after type checking passes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldConstraint {
    /// For String: minimum UTF-8 character count. For Array: minimum element count.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_length: Option<u32>,
    /// For String: maximum UTF-8 character count. For Array: maximum element count.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_length: Option<u32>,
    /// For Int/Timestamp: inclusive minimum value. Not applicable to Float.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_value: Option<i64>,
    /// For Int/Timestamp: inclusive maximum value. Not applicable to Float.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_value: Option<i64>,
    /// For String: regex pattern string. Compiled on each `validate_value` call
    /// and at registration time via `validate_schema`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pattern: Option<String>,
    /// For String: allowed values whitelist.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub enum_values: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// FieldDef
// ---------------------------------------------------------------------------

/// Single field definition within a schema.
///
/// New fields (`field_type`, `constraints`) use `#[serde(default)]` so that
/// existing serialized schemas without these fields deserialize correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    /// Name of the field.
    pub name: String,
    /// Whether the field must be present in every record.
    pub required: bool,
    /// Expected type for this field. Defaults to `FieldType::Any` for backward
    /// compatibility with data written before the schema system existed.
    #[serde(default)]
    pub field_type: FieldType,
    /// Optional constraints (length, range, pattern, enum whitelist).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub constraints: Option<FieldConstraint>,
}

// ---------------------------------------------------------------------------
// MapSchema
// ---------------------------------------------------------------------------

/// Schema definition for a map.
///
/// New fields use `#[serde(default)]` so that existing serialized schemas
/// (without `strict`) deserialize correctly with backward-compatible defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapSchema {
    /// Schema version for migrations and compatibility checks.
    pub version: u32,
    /// Field definitions that comprise this map's schema.
    pub fields: Vec<FieldDef>,
    /// If true, records with fields not defined in this schema are rejected.
    /// If false (default), extra fields are allowed.
    #[serde(default)]
    pub strict: bool,
}

// ---------------------------------------------------------------------------
// ValidationResult
// ---------------------------------------------------------------------------

/// Result of validating a value against a schema.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// The value conforms to the schema.
    Valid,
    /// The value violates one or more schema constraints.
    Invalid {
        /// Human-readable descriptions of each validation failure.
        errors: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// SyncShape
// ---------------------------------------------------------------------------

/// Defines what subset of a map's data a client receives.
/// Used for partial replication (shapes).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncShape {
    /// Client-assigned shape identifier (UUID v4).
    pub shape_id: String,
    /// Name of the map this shape applies to.
    pub map_name: String,
    /// Optional row-level filter using the `PredicateNode` expression tree.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<PredicateNode>,
    /// Optional column projection to restrict which fields are synced.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fields: Option<Vec<String>>,
    /// Optional maximum number of records to sync.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// validate_schema
// ---------------------------------------------------------------------------

/// Validate a `MapSchema` at registration time by compiling all regex patterns.
///
/// Returns `Ok(())` if every `constraints.pattern` in the schema is a valid
/// regex. Returns `Err(errors)` listing each field whose pattern fails to
/// compile. Called by `SchemaService::register_schema` to catch invalid
/// patterns before they reach the hot validation path.
///
/// # Errors
///
/// Returns `Err(errors)` where `errors` lists each invalid pattern by field name.
pub fn validate_schema(schema: &MapSchema) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = Vec::new();

    for field in &schema.fields {
        if let Some(constraints) = &field.constraints {
            if let Some(pattern) = &constraints.pattern {
                if regex::Regex::new(pattern).is_err() {
                    errors.push(format!(
                        "field '{}': invalid pattern '{pattern}'",
                        field.name
                    ));
                }
            }
        }
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

// ---------------------------------------------------------------------------
// validate_value
// ---------------------------------------------------------------------------

/// Validate a `Value` against a `MapSchema`.
///
/// The value must be a `Value::Map`. For each field definition in the schema,
/// required fields are checked for presence, typed fields are checked for type
/// compatibility, and constraints are applied. When `schema.strict` is true,
/// any field in the value that is not defined in the schema is also an error.
///
/// Regex patterns in constraints are compiled on each call. Use
/// `validate_schema` at registration time to ensure patterns are valid.
#[must_use]
pub fn validate_value(schema: &MapSchema, value: &Value) -> ValidationResult {
    let Value::Map(map) = value else {
        return ValidationResult::Invalid {
            errors: vec!["expected a Map value".to_string()],
        };
    };

    let mut errors: Vec<String> = Vec::new();

    // Check required fields and validate known fields.
    for field_def in &schema.fields {
        let field_value = map.get(&field_def.name);

        // Required check: field must be present and non-null.
        if field_def.required {
            match field_value {
                None | Some(Value::Null) => {
                    errors.push(format!("field '{}' is required", field_def.name));
                    continue; // No further checks for this field.
                }
                _ => {}
            }
        }

        // Type and constraint checks (only if the field is present and non-null).
        if let Some(val) = field_value {
            if !matches!(val, Value::Null) {
                check_type_and_constraints(val, field_def, &mut errors);
            }
        }
    }

    // Strict mode: reject fields not defined in the schema.
    if schema.strict {
        let field_defs: std::collections::HashSet<&str> =
            schema.fields.iter().map(|f| f.name.as_str()).collect();
        for key in map.keys() {
            if !field_defs.contains(key.as_str()) {
                errors.push(format!("unknown field '{key}'"));
            }
        }
    }

    if errors.is_empty() {
        ValidationResult::Valid
    } else {
        ValidationResult::Invalid { errors }
    }
}

/// Check that `value` matches `field_def.field_type` and satisfies all
/// constraints. Appends error messages to `errors`.
fn check_type_and_constraints(
    value: &Value,
    field_def: &FieldDef,
    errors: &mut Vec<String>,
) {
    let name = &field_def.name;
    let type_ok = check_type(value, &field_def.field_type, name, errors);

    if type_ok {
        if let Some(constraints) = &field_def.constraints {
            check_constraints(value, &field_def.field_type, constraints, name, errors);
        }
    }
}

/// Returns true if the value matches the expected field type (and recursively
/// checks element types for `FieldType::Array`). Appends a type error if not.
fn check_type(
    value: &Value,
    field_type: &FieldType,
    name: &str,
    errors: &mut Vec<String>,
) -> bool {
    let ok = match field_type {
        FieldType::String => matches!(value, Value::String(_)),
        // Int and Timestamp both accept Value::Int.
        FieldType::Int | FieldType::Timestamp => matches!(value, Value::Int(_)),
        // Int-to-float widening coercion: JS clients often send integers where floats are expected.
        FieldType::Float => matches!(value, Value::Float(_) | Value::Int(_)),
        FieldType::Bool => matches!(value, Value::Bool(_)),
        FieldType::Binary => matches!(value, Value::Bytes(_)),
        FieldType::Map => matches!(value, Value::Map(_)),
        FieldType::Any => !matches!(value, Value::Null),
        FieldType::Array(inner) => {
            if let Value::Array(elements) = value {
                let mut array_ok = true;
                for (i, elem) in elements.iter().enumerate() {
                    let elem_name = format!("{name}[{i}]");
                    if !check_type(elem, inner, &elem_name, errors) {
                        array_ok = false;
                    }
                }
                array_ok
            } else {
                false
            }
        }
    };

    if !ok {
        if matches!(field_type, FieldType::Array(_)) {
            // Array type mismatch (not an array at all).
            errors.push(format!(
                "field '{name}': expected Array, got {:?}",
                value_type_name(value)
            ));
        } else {
            errors.push(format!(
                "field '{name}': expected {field_type:?}, got {:?}",
                value_type_name(value)
            ));
        }
    }

    ok
}

/// Apply constraint checks (length, range, pattern, enum). Type compatibility
/// is assumed to have already passed.
fn check_constraints(
    value: &Value,
    field_type: &FieldType,
    constraints: &FieldConstraint,
    name: &str,
    errors: &mut Vec<String>,
) {
    match (value, field_type) {
        (Value::String(s), _) => {
            // Use saturating cast: strings >4 GB are rejected by max_length constraint anyway.
            #[allow(clippy::cast_possible_truncation)]
            let len = s.chars().count() as u32;
            if let Some(min) = constraints.min_length {
                if len < min {
                    errors.push(format!(
                        "field '{name}': length {len} is less than minimum {min}"
                    ));
                }
            }
            if let Some(max) = constraints.max_length {
                if len > max {
                    errors.push(format!(
                        "field '{name}': length {len} exceeds maximum {max}"
                    ));
                }
            }
            if let Some(pattern) = &constraints.pattern {
                match regex::Regex::new(pattern) {
                    Ok(re) => {
                        if !re.is_match(s) {
                            errors.push(format!(
                                "field '{name}': value does not match pattern '{pattern}'"
                            ));
                        }
                    }
                    Err(_) => {
                        errors.push(format!("field '{name}': invalid pattern '{pattern}'"));
                    }
                }
            }
            if let Some(enum_values) = &constraints.enum_values {
                if !enum_values.iter().any(|v| v == s) {
                    errors.push(format!(
                        "field '{name}': value '{s}' is not in the allowed list"
                    ));
                }
            }
        }
        (Value::Array(arr), _) => {
            // Use saturating cast: arrays >4 GB are rejected by max_length constraint anyway.
            #[allow(clippy::cast_possible_truncation)]
            let len = arr.len() as u32;
            if let Some(min) = constraints.min_length {
                if len < min {
                    errors.push(format!(
                        "field '{name}': array length {len} is less than minimum {min}"
                    ));
                }
            }
            if let Some(max) = constraints.max_length {
                if len > max {
                    errors.push(format!(
                        "field '{name}': array length {len} exceeds maximum {max}"
                    ));
                }
            }
        }
        (Value::Int(i), FieldType::Int | FieldType::Timestamp) => {
            if let Some(min) = constraints.min_value {
                if *i < min {
                    errors.push(format!(
                        "field '{name}': value {i} is less than minimum {min}"
                    ));
                }
            }
            if let Some(max) = constraints.max_value {
                if *i > max {
                    errors.push(format!(
                        "field '{name}': value {i} exceeds maximum {max}"
                    ));
                }
            }
        }
        // Int coerced to Float — range constraints do not apply (see Known Limitations).
        _ => {}
    }
}

/// Returns a human-readable type name for a `Value` variant.
fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::String(_) => "String",
        Value::Bytes(_) => "Bytes",
        Value::Array(_) => "Array",
        Value::Map(_) => "Map",
    }
}

// ---------------------------------------------------------------------------
// Arrow schema conversion (enabled by the `arrow` feature flag)
// ---------------------------------------------------------------------------

#[cfg(feature = "arrow")]
impl MapSchema {
    /// Derive an Apache Arrow `Schema` from this map schema.
    ///
    /// Each `FieldDef` becomes an Arrow `Field` with the corresponding
    /// `DataType`. The `required` flag maps to Arrow nullability (required
    /// fields are non-nullable).
    #[must_use]
    pub fn to_arrow_schema(&self) -> arrow_schema::Schema {
        use arrow_schema::{Field, Schema};
        let fields: Vec<Field> = self
            .fields
            .iter()
            .map(|fd| {
                let data_type = field_type_to_arrow(&fd.field_type);
                // Arrow nullable = true means the field may be null.
                // TopGun required = true means the field must not be null,
                // so we invert: required fields are non-nullable in Arrow.
                let nullable = !fd.required;
                Field::new(&fd.name, data_type, nullable)
            })
            .collect();
        Schema::new(fields)
    }
}

/// Convert a `FieldType` to the corresponding Arrow `DataType`.
///
/// Handles the recursive `Array` case by building a `List` type with a
/// nullable `item` field for each element.
#[cfg(feature = "arrow")]
fn field_type_to_arrow(ft: &FieldType) -> arrow_schema::DataType {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, TimeUnit};

    match ft {
        // String, nested maps, and untyped fields all surface as UTF-8 text
        // in Arrow. Nested maps are opaque in v1 (no recursive schema) and
        // are serialized as JSON strings for SQL queryability.
        FieldType::String | FieldType::Map | FieldType::Any => DataType::Utf8,
        FieldType::Int => DataType::Int64,
        FieldType::Float => DataType::Float64,
        FieldType::Bool => DataType::Boolean,
        FieldType::Binary => DataType::Binary,
        FieldType::Timestamp => DataType::Timestamp(TimeUnit::Millisecond, None),
        FieldType::Array(inner) => {
            let inner_type = field_type_to_arrow(inner);
            // Array elements are always nullable to allow optional entries.
            let item_field = Arc::new(Field::new("item", inner_type, true));
            DataType::List(item_field)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn make_string_field(name: &str, required: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            required,
            field_type: FieldType::String,
            constraints: None,
        }
    }

    fn make_schema(fields: Vec<FieldDef>, strict: bool) -> MapSchema {
        MapSchema { version: 1, fields, strict }
    }

    fn make_map(pairs: Vec<(&str, Value)>) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v);
        }
        Value::Map(m)
    }

    // -----------------------------------------------------------------------
    // validate_value: basic cases
    // -----------------------------------------------------------------------

    #[test]
    fn valid_map_passes() {
        let schema = make_schema(vec![make_string_field("name", true)], false);
        let value = make_map(vec![("name", Value::String("Alice".to_string()))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn non_map_value_returns_invalid() {
        let schema = make_schema(vec![], false);
        let result = validate_value(&schema, &Value::String("not a map".to_string()));
        assert!(matches!(result, ValidationResult::Invalid { .. }));
        if let ValidationResult::Invalid { errors } = result {
            assert!(errors[0].contains("expected a Map value"));
        }
    }

    #[test]
    fn missing_required_field_returns_invalid() {
        let schema = make_schema(vec![make_string_field("name", true)], false);
        let value = make_map(vec![]);
        let result = validate_value(&schema, &value);
        assert!(matches!(result, ValidationResult::Invalid { .. }));
        if let ValidationResult::Invalid { errors } = result {
            assert!(errors[0].contains("field 'name' is required"));
        }
    }

    #[test]
    fn null_required_field_returns_invalid() {
        let schema = make_schema(vec![make_string_field("name", true)], false);
        let value = make_map(vec![("name", Value::Null)]);
        let result = validate_value(&schema, &value);
        assert!(matches!(result, ValidationResult::Invalid { .. }));
    }

    #[test]
    fn wrong_type_returns_invalid() {
        let schema = make_schema(vec![make_string_field("age", false)], false);
        let value = make_map(vec![("age", Value::Int(42))]);
        let result = validate_value(&schema, &value);
        assert!(matches!(result, ValidationResult::Invalid { .. }));
    }

    #[test]
    fn optional_field_absent_is_valid() {
        let schema = make_schema(vec![make_string_field("name", false)], false);
        let value = make_map(vec![]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    // -----------------------------------------------------------------------
    // validate_value: type rules
    // -----------------------------------------------------------------------

    #[test]
    fn int_type_accepts_int() {
        let schema = make_schema(
            vec![FieldDef {
                name: "x".to_string(),
                required: false,
                field_type: FieldType::Int,
                constraints: None,
            }],
            false,
        );
        let value = make_map(vec![("x", Value::Int(5))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn float_type_accepts_int_widening() {
        let schema = make_schema(
            vec![FieldDef {
                name: "x".to_string(),
                required: false,
                field_type: FieldType::Float,
                constraints: None,
            }],
            false,
        );
        let value = make_map(vec![("x", Value::Int(5))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn float_type_accepts_float() {
        let schema = make_schema(
            vec![FieldDef {
                name: "x".to_string(),
                required: false,
                field_type: FieldType::Float,
                constraints: None,
            }],
            false,
        );
        let value = make_map(vec![("x", Value::Float(std::f64::consts::PI))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn binary_type_accepts_bytes() {
        let schema = make_schema(
            vec![FieldDef {
                name: "data".to_string(),
                required: false,
                field_type: FieldType::Binary,
                constraints: None,
            }],
            false,
        );
        let value = make_map(vec![("data", Value::Bytes(vec![0xDE, 0xAD]))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn timestamp_type_accepts_int() {
        let schema = make_schema(
            vec![FieldDef {
                name: "ts".to_string(),
                required: false,
                field_type: FieldType::Timestamp,
                constraints: None,
            }],
            false,
        );
        let value = make_map(vec![("ts", Value::Int(1_700_000_000_000))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn bool_type_accepts_bool() {
        let schema = make_schema(
            vec![FieldDef {
                name: "active".to_string(),
                required: false,
                field_type: FieldType::Bool,
                constraints: None,
            }],
            false,
        );
        let value = make_map(vec![("active", Value::Bool(true))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    #[test]
    fn any_type_accepts_non_null() {
        let schema = make_schema(
            vec![FieldDef {
                name: "x".to_string(),
                required: false,
                field_type: FieldType::Any,
                constraints: None,
            }],
            false,
        );
        for val in [
            Value::Bool(true),
            Value::Int(1),
            Value::Float(1.0),
            Value::String("s".to_string()),
            Value::Bytes(vec![1]),
        ] {
            let value = make_map(vec![("x", val)]);
            assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
        }
    }

    #[test]
    fn array_type_checks_element_types() {
        let schema = make_schema(
            vec![FieldDef {
                name: "tags".to_string(),
                required: false,
                field_type: FieldType::Array(Box::new(FieldType::String)),
                constraints: None,
            }],
            false,
        );
        // Valid: all strings.
        let valid = make_map(vec![(
            "tags",
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ]),
        )]);
        assert!(matches!(validate_value(&schema, &valid), ValidationResult::Valid));

        // Invalid: contains an int.
        let invalid = make_map(vec![(
            "tags",
            Value::Array(vec![Value::String("a".to_string()), Value::Int(1)]),
        )]);
        assert!(matches!(validate_value(&schema, &invalid), ValidationResult::Invalid { .. }));
    }

    // -----------------------------------------------------------------------
    // validate_value: constraints
    // -----------------------------------------------------------------------

    #[test]
    fn max_length_string_constraint() {
        let schema = make_schema(
            vec![FieldDef {
                name: "name".to_string(),
                required: false,
                field_type: FieldType::String,
                constraints: Some(FieldConstraint {
                    max_length: Some(10),
                    ..Default::default()
                }),
            }],
            false,
        );
        // 11-char string — exceeds max.
        let value = make_map(vec![("name", Value::String("hello world".to_string()))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Invalid { .. }));

        // 5-char string — within limit.
        let value2 = make_map(vec![("name", Value::String("hello".to_string()))]);
        assert!(matches!(validate_value(&schema, &value2), ValidationResult::Valid));
    }

    #[test]
    fn min_value_int_constraint() {
        let schema = make_schema(
            vec![FieldDef {
                name: "age".to_string(),
                required: false,
                field_type: FieldType::Int,
                constraints: Some(FieldConstraint {
                    min_value: Some(0),
                    ..Default::default()
                }),
            }],
            false,
        );
        let invalid = make_map(vec![("age", Value::Int(-1))]);
        assert!(matches!(validate_value(&schema, &invalid), ValidationResult::Invalid { .. }));

        let valid = make_map(vec![("age", Value::Int(0))]);
        assert!(matches!(validate_value(&schema, &valid), ValidationResult::Valid));
    }

    #[test]
    fn pattern_constraint_valid_string() {
        let schema = make_schema(
            vec![FieldDef {
                name: "email".to_string(),
                required: false,
                field_type: FieldType::String,
                constraints: Some(FieldConstraint {
                    pattern: Some(r"^\S+@\S+\.\S+$".to_string()),
                    ..Default::default()
                }),
            }],
            false,
        );
        let valid = make_map(vec![("email", Value::String("user@example.com".to_string()))]);
        assert!(matches!(validate_value(&schema, &valid), ValidationResult::Valid));

        let invalid = make_map(vec![("email", Value::String("not-an-email".to_string()))]);
        assert!(matches!(validate_value(&schema, &invalid), ValidationResult::Invalid { .. }));
    }

    #[test]
    fn enum_values_constraint() {
        let schema = make_schema(
            vec![FieldDef {
                name: "status".to_string(),
                required: false,
                field_type: FieldType::String,
                constraints: Some(FieldConstraint {
                    enum_values: Some(vec!["active".to_string(), "inactive".to_string()]),
                    ..Default::default()
                }),
            }],
            false,
        );
        let valid = make_map(vec![("status", Value::String("active".to_string()))]);
        assert!(matches!(validate_value(&schema, &valid), ValidationResult::Valid));

        let invalid = make_map(vec![("status", Value::String("pending".to_string()))]);
        assert!(matches!(validate_value(&schema, &invalid), ValidationResult::Invalid { .. }));
    }

    // -----------------------------------------------------------------------
    // validate_value: strict mode
    // -----------------------------------------------------------------------

    #[test]
    fn strict_mode_rejects_unknown_fields() {
        let schema = make_schema(vec![make_string_field("name", false)], true);
        let value = make_map(vec![
            ("name", Value::String("Alice".to_string())),
            ("unknown", Value::Int(99)),
        ]);
        let result = validate_value(&schema, &value);
        assert!(matches!(result, ValidationResult::Invalid { .. }));
        if let ValidationResult::Invalid { errors } = result {
            assert!(errors.iter().any(|e| e.contains("unknown field 'unknown'")));
        }
    }

    #[test]
    fn non_strict_mode_allows_extra_fields() {
        let schema = make_schema(vec![make_string_field("name", false)], false);
        let value = make_map(vec![
            ("name", Value::String("Alice".to_string())),
            ("extra", Value::Int(42)),
        ]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Valid));
    }

    // -----------------------------------------------------------------------
    // validate_schema
    // -----------------------------------------------------------------------

    #[test]
    fn validate_schema_ok_for_valid_patterns() {
        let schema = make_schema(
            vec![FieldDef {
                name: "email".to_string(),
                required: false,
                field_type: FieldType::String,
                constraints: Some(FieldConstraint {
                    pattern: Some(r"^\S+@\S+$".to_string()),
                    ..Default::default()
                }),
            }],
            false,
        );
        assert!(validate_schema(&schema).is_ok());
    }

    #[test]
    fn validate_schema_err_for_invalid_pattern() {
        let schema = make_schema(
            vec![FieldDef {
                name: "bad".to_string(),
                required: false,
                field_type: FieldType::String,
                constraints: Some(FieldConstraint {
                    pattern: Some("[invalid".to_string()),
                    ..Default::default()
                }),
            }],
            false,
        );
        let result = validate_schema(&schema);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors[0].contains("bad"));
        assert!(errors[0].contains("[invalid"));
    }

    #[test]
    fn validate_schema_no_constraints_is_ok() {
        let schema = make_schema(vec![make_string_field("name", true)], false);
        assert!(validate_schema(&schema).is_ok());
    }

    // -----------------------------------------------------------------------
    // Backward compatibility: deserialize old-format schema
    // -----------------------------------------------------------------------

    #[test]
    fn old_format_schema_deserializes_with_defaults() {
        // An old MapSchema serialized without field_type, constraints, or strict.
        let old_json = r#"{"version":1,"fields":[{"name":"foo","required":true}]}"#;
        let schema: MapSchema = serde_json::from_str(old_json).expect("deserialize");
        assert_eq!(schema.version, 1);
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].field_type, FieldType::Any);
        assert!(schema.fields[0].constraints.is_none());
        assert!(!schema.strict);
    }

    // -----------------------------------------------------------------------
    // MsgPack round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn map_schema_msgpack_roundtrip() {
        let schema = MapSchema {
            version: 2,
            strict: true,
            fields: vec![
                FieldDef {
                    name: "name".to_string(),
                    required: true,
                    field_type: FieldType::String,
                    constraints: Some(FieldConstraint {
                        max_length: Some(100),
                        ..Default::default()
                    }),
                },
                FieldDef {
                    name: "age".to_string(),
                    required: false,
                    field_type: FieldType::Int,
                    constraints: Some(FieldConstraint {
                        min_value: Some(0),
                        max_value: Some(150),
                        ..Default::default()
                    }),
                },
            ],
        };
        let bytes = rmp_serde::to_vec_named(&schema).expect("serialize");
        let decoded: MapSchema = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(decoded.version, 2);
        assert!(decoded.strict);
        assert_eq!(decoded.fields.len(), 2);
        assert_eq!(decoded.fields[0].field_type, FieldType::String);
        assert_eq!(decoded.fields[1].field_type, FieldType::Int);
    }

    // -----------------------------------------------------------------------
    // Acceptance criterion 3: strict mode + required + max_length
    // -----------------------------------------------------------------------

    #[test]
    fn strict_required_max_length_scenario() {
        let schema = MapSchema {
            version: 1,
            strict: true,
            fields: vec![FieldDef {
                name: "title".to_string(),
                required: true,
                field_type: FieldType::String,
                constraints: Some(FieldConstraint {
                    max_length: Some(10),
                    ..Default::default()
                }),
            }],
        };
        // 11-char string — fails max_length.
        let value = make_map(vec![("title", Value::String("hello world".to_string()))]);
        assert!(matches!(validate_value(&schema, &value), ValidationResult::Invalid { .. }));
    }

    // -----------------------------------------------------------------------
    // to_arrow_schema: one test per FieldType variant + nullability + array + round-trip
    // -----------------------------------------------------------------------

    #[cfg(feature = "arrow")]
    mod arrow_tests {
        use arrow_schema::{DataType, TimeUnit};

        use super::*;

        fn make_field(name: &str, ft: FieldType, required: bool) -> FieldDef {
            FieldDef { name: name.to_string(), required, field_type: ft, constraints: None }
        }

        #[test]
        fn string_maps_to_utf8() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("s", FieldType::String, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Utf8);
        }

        #[test]
        fn int_maps_to_int64() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("i", FieldType::Int, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Int64);
        }

        #[test]
        fn float_maps_to_float64() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("f", FieldType::Float, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Float64);
        }

        #[test]
        fn bool_maps_to_boolean() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("b", FieldType::Bool, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Boolean);
        }

        #[test]
        fn binary_maps_to_binary() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("bin", FieldType::Binary, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Binary);
        }

        #[test]
        fn timestamp_maps_to_timestamp_millisecond_no_tz() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("ts", FieldType::Timestamp, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(
                arrow.field(0).data_type(),
                &DataType::Timestamp(TimeUnit::Millisecond, None)
            );
        }

        #[test]
        fn map_variant_maps_to_utf8() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("m", FieldType::Map, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Utf8);
        }

        #[test]
        fn any_variant_maps_to_utf8() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("a", FieldType::Any, false)] };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.field(0).data_type(), &DataType::Utf8);
        }

        #[test]
        fn array_of_int_maps_to_list_int64() {
            let schema = MapSchema {
                version: 1,
                strict: false,
                fields: vec![make_field("ids", FieldType::Array(Box::new(FieldType::Int)), false)],
            };
            let arrow = schema.to_arrow_schema();
            let dt = arrow.field(0).data_type();
            if let DataType::List(item_field) = dt {
                assert_eq!(item_field.data_type(), &DataType::Int64);
                assert!(item_field.is_nullable(), "array item field should be nullable");
            } else {
                panic!("expected DataType::List, got {dt:?}");
            }
        }

        #[test]
        fn required_true_produces_non_nullable_field() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("name", FieldType::String, true)] };
            let arrow = schema.to_arrow_schema();
            assert!(!arrow.field(0).is_nullable(), "required field must be non-nullable in Arrow");
        }

        #[test]
        fn required_false_produces_nullable_field() {
            let schema =
                MapSchema { version: 1, strict: false, fields: vec![make_field("name", FieldType::String, false)] };
            let arrow = schema.to_arrow_schema();
            assert!(arrow.field(0).is_nullable(), "optional field must be nullable in Arrow");
        }

        #[test]
        fn nested_array_of_string_produces_list_utf8() {
            let schema = MapSchema {
                version: 1,
                strict: false,
                fields: vec![make_field("tags", FieldType::Array(Box::new(FieldType::String)), false)],
            };
            let arrow = schema.to_arrow_schema();
            if let DataType::List(item_field) = arrow.field(0).data_type() {
                assert_eq!(item_field.data_type(), &DataType::Utf8);
            } else {
                panic!("expected DataType::List");
            }
        }

        #[test]
        fn multi_field_schema_round_trip() {
            let schema = MapSchema {
                version: 1,
                strict: false,
                fields: vec![
                    make_field("id", FieldType::String, true),
                    make_field("age", FieldType::Int, false),
                    make_field("score", FieldType::Float, false),
                    make_field("active", FieldType::Bool, true),
                    make_field("data", FieldType::Binary, false),
                    make_field("created_at", FieldType::Timestamp, false),
                    make_field("tags", FieldType::Array(Box::new(FieldType::String)), false),
                    make_field("meta", FieldType::Map, false),
                    make_field("anything", FieldType::Any, false),
                ],
            };
            let arrow = schema.to_arrow_schema();
            assert_eq!(arrow.fields().len(), 9);

            // Spot-check a few fields.
            assert_eq!(arrow.field(0).name(), "id");
            assert!(!arrow.field(0).is_nullable());
            assert_eq!(arrow.field(0).data_type(), &DataType::Utf8);

            assert_eq!(arrow.field(1).name(), "age");
            assert!(arrow.field(1).is_nullable());
            assert_eq!(arrow.field(1).data_type(), &DataType::Int64);

            assert_eq!(arrow.field(6).name(), "tags");
            assert!(matches!(arrow.field(6).data_type(), DataType::List(_)));
        }
    }
}
