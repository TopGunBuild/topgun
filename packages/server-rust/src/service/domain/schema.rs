use async_trait::async_trait;
use dashmap::DashMap;
use topgun_core::{
    validate_schema, validate_value, MapSchema, RequestContext, SyncShape, ValidationResult, Value,
};

use crate::service::registry::{ManagedService, ServiceContext};
use crate::traits::SchemaProvider;

// ---------------------------------------------------------------------------
// SchemaService
// ---------------------------------------------------------------------------

/// Stores map schemas and validates incoming CRDT writes against typed field
/// definitions. Implements the `SchemaProvider` trait.
///
/// Schemas are registered infrequently but read on every write, so `DashMap`
/// provides efficient concurrent read access with minimal write contention.
///
/// Optional mode: maps with no registered schema pass validation automatically.
pub struct SchemaService {
    schemas: DashMap<String, MapSchema>,
}

impl SchemaService {
    /// Creates a new `SchemaService` with an empty schema registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schemas: DashMap::new(),
        }
    }
}

impl Default for SchemaService {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ManagedService impl
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for SchemaService {
    fn name(&self) -> &'static str {
        "schema"
    }

    async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn reset(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SchemaProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl SchemaProvider for SchemaService {
    async fn get_schema(&self, map_name: &str) -> Option<MapSchema> {
        self.schemas
            .get(map_name)
            .map(|entry| entry.value().clone())
    }

    /// Register a schema for a map.
    ///
    /// Calls `validate_schema` first to ensure all regex patterns compile. If
    /// any pattern is invalid, registration is rejected and an error is returned.
    /// On success, any existing schema for the map is overwritten.
    async fn register_schema(&self, map_name: &str, schema: MapSchema) -> anyhow::Result<()> {
        if let Err(errors) = validate_schema(&schema) {
            return Err(anyhow::anyhow!(
                "schema for '{}' contains invalid patterns: {}",
                map_name,
                errors.join("; ")
            ));
        }
        self.schemas.insert(map_name.to_string(), schema);
        Ok(())
    }

    /// Validate a value against the registered schema for the given map.
    ///
    /// Returns `Valid` if no schema is registered (optional mode: no schema
    /// means no validation). Delegates to `validate_value` when a schema exists.
    fn validate(&self, map_name: &str, value: &Value) -> ValidationResult {
        match self.schemas.get(map_name) {
            None => ValidationResult::Valid,
            Some(schema) => validate_value(schema.value(), value),
        }
    }

    /// Compute the sync shape for a client.
    ///
    /// Returns `None` — shape computation is deferred to TODO-070.
    async fn get_shape(&self, _map_name: &str, _client_ctx: &RequestContext) -> Option<SyncShape> {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use topgun_core::{FieldConstraint, FieldDef, FieldType, ValidationResult, Value};

    use super::*;

    fn make_map(pairs: Vec<(&str, Value)>) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v);
        }
        Value::Map(m)
    }

    fn string_field(name: &str, required: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            required,
            field_type: FieldType::String,
            constraints: None,
        }
    }

    fn make_schema(fields: Vec<FieldDef>) -> MapSchema {
        MapSchema {
            version: 1,
            fields,
            strict: false,
        }
    }

    // -----------------------------------------------------------------------
    // register_schema / get_schema
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn register_and_retrieve_schema() {
        let svc = SchemaService::new();
        let schema = make_schema(vec![string_field("name", true)]);
        svc.register_schema("users", schema.clone()).await.unwrap();

        let retrieved = svc.get_schema("users").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().version, 1);
    }

    #[tokio::test]
    async fn get_schema_unregistered_returns_none() {
        let svc = SchemaService::new();
        assert!(svc.get_schema("missing").await.is_none());
    }

    #[tokio::test]
    async fn register_schema_overwrites_existing() {
        let svc = SchemaService::new();
        let v1 = MapSchema {
            version: 1,
            fields: vec![],
            strict: false,
        };
        let v2 = MapSchema {
            version: 2,
            fields: vec![],
            strict: false,
        };
        svc.register_schema("map", v1).await.unwrap();
        svc.register_schema("map", v2).await.unwrap();
        assert_eq!(svc.get_schema("map").await.unwrap().version, 2);
    }

    #[tokio::test]
    async fn register_schema_rejects_invalid_pattern() {
        let svc = SchemaService::new();
        let schema = make_schema(vec![FieldDef {
            name: "x".to_string(),
            required: false,
            field_type: FieldType::String,
            constraints: Some(FieldConstraint {
                pattern: Some("[invalid".to_string()),
                ..Default::default()
            }),
        }]);
        let result = svc.register_schema("test", schema).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("[invalid"));
    }

    // -----------------------------------------------------------------------
    // validate (optional mode + delegation)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_no_schema_returns_valid() {
        let svc = SchemaService::new();
        let value = make_map(vec![("x", Value::Int(1))]);
        assert!(matches!(
            svc.validate("any_map", &value),
            ValidationResult::Valid
        ));
    }

    #[tokio::test]
    async fn validate_conforming_value_returns_valid() {
        let svc = SchemaService::new();
        let schema = make_schema(vec![string_field("name", true)]);
        svc.register_schema("users", schema).await.unwrap();

        let value = make_map(vec![("name", Value::String("Alice".to_string()))]);
        assert!(matches!(
            svc.validate("users", &value),
            ValidationResult::Valid
        ));
    }

    #[tokio::test]
    async fn validate_missing_required_field_returns_invalid() {
        let svc = SchemaService::new();
        let schema = make_schema(vec![string_field("name", true)]);
        svc.register_schema("users", schema).await.unwrap();

        let value = make_map(vec![]);
        assert!(matches!(
            svc.validate("users", &value),
            ValidationResult::Invalid { .. }
        ));
    }

    #[tokio::test]
    async fn validate_wrong_type_returns_invalid() {
        let svc = SchemaService::new();
        let schema = make_schema(vec![string_field("name", false)]);
        svc.register_schema("users", schema).await.unwrap();

        let value = make_map(vec![("name", Value::Int(42))]);
        assert!(matches!(
            svc.validate("users", &value),
            ValidationResult::Invalid { .. }
        ));
    }

    #[tokio::test]
    async fn validate_with_max_length_constraint() {
        let svc = SchemaService::new();
        let schema = make_schema(vec![FieldDef {
            name: "title".to_string(),
            required: true,
            field_type: FieldType::String,
            constraints: Some(FieldConstraint {
                max_length: Some(10),
                ..Default::default()
            }),
        }]);
        svc.register_schema("items", schema).await.unwrap();

        // 11-char string fails.
        let invalid = make_map(vec![("title", Value::String("hello world".to_string()))]);
        assert!(matches!(
            svc.validate("items", &invalid),
            ValidationResult::Invalid { .. }
        ));

        // 5-char string passes.
        let valid = make_map(vec![("title", Value::String("hello".to_string()))]);
        assert!(matches!(
            svc.validate("items", &valid),
            ValidationResult::Valid
        ));
    }

    // -----------------------------------------------------------------------
    // get_shape (stub)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_shape_returns_none() {
        let svc = SchemaService::new();
        let ctx = RequestContext {
            node_id: "test".to_string(),
            tenant_id: None,
            principal: None,
            trace_id: "trace-1".to_string(),
        };
        assert!(svc.get_shape("any_map", &ctx).await.is_none());
    }

    // -----------------------------------------------------------------------
    // ManagedService lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn lifecycle_noop_methods_return_ok() {
        use crate::service::config::ServerConfig;
        use crate::service::registry::ServiceContext;
        use std::sync::Arc;

        let svc = SchemaService::new();
        let ctx = ServiceContext {
            config: Arc::new(ServerConfig::default()),
        };

        assert_eq!(svc.name(), "schema");
        svc.init(&ctx).await.unwrap();
        svc.reset().await.unwrap();
        svc.shutdown(false).await.unwrap();
        svc.shutdown(true).await.unwrap();
    }
}
