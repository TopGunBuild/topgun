use serde::{Deserialize, Serialize};

/// Schema definition for a map. Placeholder: will carry field definitions,
/// version info, and validation rules when the schema system is built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapSchema {
    /// Schema version for migrations and compatibility checks.
    pub version: u32,
    /// Field definitions that comprise this map's schema.
    pub fields: Vec<FieldDef>,
}

/// Single field definition within a schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    /// Name of the field.
    pub name: String,
    /// Whether the field must be present in every record.
    pub required: bool,
}

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

/// Row-level filter predicate for sync shapes.
/// Placeholder: will become an expression tree when query filtering is built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predicate {
    /// String representation of the filter expression.
    pub expression: String,
}

/// Defines what subset of a map's data a client receives.
/// Used for partial replication (shapes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncShape {
    /// Name of the map this shape applies to.
    pub map_name: String,
    /// Optional row-level filter to restrict which records are synced.
    pub filter: Option<Predicate>,
    /// Optional column projection to restrict which fields are synced.
    pub fields: Option<Vec<String>>,
    /// Optional maximum number of records to sync.
    pub limit: Option<usize>,
}
