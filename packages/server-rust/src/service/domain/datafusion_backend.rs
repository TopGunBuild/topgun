//! `DataFusion`-backed query engine implementation.
//!
//! `DataFusionBackend` implements both `QueryBackend` (predicate queries) and
//! `SqlQueryBackend` (SQL queries). Predicate queries delegate to the existing
//! predicate engine for backward compatibility. SQL queries are executed via
//! `DataFusion`'s `SessionContext`.
//!
//! All types in this module are feature-gated behind `#[cfg(feature = "datafusion")]`.

use std::sync::Arc;

use arrow::array::RecordBatch;
use async_trait::async_trait;
use datafusion::prelude::SessionContext;
use topgun_core::messages::{Query, QueryResultEntry};

use super::arrow_cache::ArrowCacheManager;
use super::arrow_convert::make_arrow_schema;
use super::query_backend::{QueryBackend, QueryBackendError, SqlQueryBackend};
use super::table_provider::TopGunTableProvider;
use crate::storage::RecordStoreFactory;
use crate::traits::SchemaProvider;

// ---------------------------------------------------------------------------
// DataFusionBackend
// ---------------------------------------------------------------------------

/// Query backend powered by Apache `DataFusion`.
///
/// Supports both predicate-based queries (delegated to the existing predicate
/// engine) and SQL queries executed via `DataFusion`'s `SessionContext`.
///
/// Maps are registered as `DataFusion` tables via `register_map()`, which
/// creates a `TopGunTableProvider` backed by the `RecordStoreFactory` and
/// `ArrowCacheManager`.
pub struct DataFusionBackend {
    ctx: SessionContext,
    record_store_factory: Arc<RecordStoreFactory>,
    schema_provider: Arc<dyn SchemaProvider>,
    cache_manager: Arc<ArrowCacheManager>,
}

impl DataFusionBackend {
    /// Creates a new `DataFusion` backend.
    ///
    /// The `schema_provider` is used to look up `MapSchema` for maps when
    /// registering them as `DataFusion` tables. The `cache_manager` is shared
    /// with `ArrowCacheObserver` instances for cache invalidation.
    #[must_use]
    pub fn new(
        record_store_factory: Arc<RecordStoreFactory>,
        schema_provider: Arc<dyn SchemaProvider>,
        cache_manager: Arc<ArrowCacheManager>,
    ) -> Self {
        Self {
            ctx: SessionContext::new(),
            record_store_factory,
            schema_provider,
            cache_manager,
        }
    }
}

#[async_trait]
impl QueryBackend for DataFusionBackend {
    /// Delegates to the existing predicate engine for backward compatibility.
    ///
    /// `DataFusion` is not used for predicate-based queries. This ensures that
    /// standing query subscriptions continue to work without schema registration.
    async fn execute_query(
        &self,
        _map_name: &str,
        entries: Vec<(String, rmpv::Value)>,
        query: &Query,
    ) -> Result<Vec<QueryResultEntry>, QueryBackendError> {
        Ok(super::predicate::execute_query(entries, query))
    }

    /// Registers a map as a `DataFusion` table.
    ///
    /// Looks up the `MapSchema` from the `SchemaProvider`, builds an Arrow
    /// schema with prepended `_key` column, creates a `TopGunTableProvider`,
    /// and registers it with the `DataFusion` `SessionContext`.
    async fn register_map(&self, map_name: &str) -> Result<(), QueryBackendError> {
        let map_schema = self
            .schema_provider
            .get_schema(map_name)
            .await
            .ok_or_else(|| QueryBackendError::SchemaRequired(map_name.to_string()))?;

        let arrow_schema = Arc::new(make_arrow_schema(&map_schema));
        let provider = TopGunTableProvider::new(
            map_name.to_string(),
            arrow_schema,
            Arc::clone(&self.record_store_factory),
            Arc::clone(&self.cache_manager),
        );

        self.ctx
            .register_table(map_name, Arc::new(provider))
            .map_err(|e| QueryBackendError::Execution(e.to_string()))?;

        Ok(())
    }

    /// Deregisters a map from the `DataFusion` `SessionContext`.
    async fn deregister_map(&self, map_name: &str) -> Result<(), QueryBackendError> {
        self.ctx
            .deregister_table(map_name)
            .map_err(|e| QueryBackendError::Execution(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl SqlQueryBackend for DataFusionBackend {
    /// Executes a SQL query string via `DataFusion`, returning Arrow `RecordBatches`.
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<RecordBatch>, QueryBackendError> {
        let df = self
            .ctx
            .sql(sql)
            .await
            .map_err(|e| QueryBackendError::SqlParse(e.to_string()))?;

        let batches = df
            .collect()
            .await
            .map_err(|e| QueryBackendError::Execution(e.to_string()))?;

        Ok(batches)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, Int64Array, StringArray};
    use std::collections::BTreeMap;
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;
    use topgun_core::{FieldDef, FieldType, MapSchema};

    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::{CallerProvenance, ExpiryPolicy};
    use crate::storage::record::RecordValue;

    /// Test schema provider that returns a fixed schema for "users".
    struct TestSchemaProvider {
        schemas: dashmap::DashMap<String, MapSchema>,
    }

    impl TestSchemaProvider {
        fn new() -> Self {
            Self {
                schemas: dashmap::DashMap::new(),
            }
        }

        fn register(&self, name: &str, schema: MapSchema) {
            self.schemas.insert(name.to_string(), schema);
        }
    }

    #[async_trait]
    impl SchemaProvider for TestSchemaProvider {
        async fn get_schema(&self, map_name: &str) -> Option<MapSchema> {
            self.schemas.get(map_name).map(|e| e.value().clone())
        }

        async fn register_schema(&self, map_name: &str, schema: MapSchema) -> anyhow::Result<()> {
            self.schemas.insert(map_name.to_string(), schema);
            Ok(())
        }

        fn validate(&self, _map_name: &str, _value: &topgun_core::types::Value) -> topgun_core::ValidationResult {
            topgun_core::ValidationResult::Valid
        }

        async fn get_shape(
            &self,
            _map_name: &str,
            _client_ctx: &topgun_core::RequestContext,
        ) -> Option<topgun_core::SyncShape> {
            None
        }
    }

    fn users_schema() -> MapSchema {
        MapSchema {
            version: 1,
            strict: false,
            fields: vec![
                FieldDef {
                    name: "name".to_string(),
                    required: true,
                    field_type: FieldType::String,
                    constraints: None,
                },
                FieldDef {
                    name: "age".to_string(),
                    required: false,
                    field_type: FieldType::Int,
                    constraints: None,
                },
            ],
        }
    }

    fn test_timestamp() -> Timestamp {
        Timestamp {
            millis: 1000,
            counter: 0,
            node_id: "test".to_string(),
        }
    }

    fn make_user(name: &str, age: i64) -> Value {
        let mut m = BTreeMap::new();
        m.insert("name".to_string(), Value::String(name.to_string()));
        m.insert("age".to_string(), Value::Int(age));
        Value::Map(m)
    }

    fn setup_backend() -> (DataFusionBackend, Arc<RecordStoreFactory>) {
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let sp = Arc::new(TestSchemaProvider::new());
        sp.register("users", users_schema());
        let cache = Arc::new(ArrowCacheManager::new());
        let backend = DataFusionBackend::new(
            Arc::clone(&factory),
            sp,
            cache,
        );
        (backend, factory)
    }

    async fn populate_users(factory: &RecordStoreFactory, users: Vec<(&str, &str, i64)>) {
        for (key, name, age) in users {
            let store = factory.get_or_create("users", 0);
            store.put(
                key,
                RecordValue::Lww {
                    value: make_user(name, age),
                    timestamp: test_timestamp(),
                },
                ExpiryPolicy::NONE,
                CallerProvenance::Client,
            ).await.unwrap();
        }
    }

    #[tokio::test]
    async fn select_all_returns_all_rows() {
        let (backend, factory) = setup_backend();
        populate_users(
            &factory,
            vec![("u1", "Alice", 30), ("u2", "Bob", 25), ("u3", "Charlie", 35)],
        ).await;

        backend.register_map("users").await.unwrap();
        let batches = backend.execute_sql("SELECT * FROM users").await.unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3);
    }

    #[tokio::test]
    async fn select_with_where_filters_correctly() {
        let (backend, factory) = setup_backend();
        populate_users(
            &factory,
            vec![("u1", "Alice", 30), ("u2", "Bob", 25), ("u3", "Charlie", 35)],
        ).await;

        backend.register_map("users").await.unwrap();
        let batches = backend
            .execute_sql("SELECT _key, name, age FROM users WHERE age > 25")
            .await
            .unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        // All returned rows should have age > 25.
        for batch in &batches {
            let ages = batch
                .column(2)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            for i in 0..batch.num_rows() {
                assert!(ages.value(i) > 25);
            }
        }
    }

    #[tokio::test]
    async fn select_with_group_by_aggregates() {
        let (backend, factory) = setup_backend();
        populate_users(
            &factory,
            vec![
                ("u1", "Alice", 30),
                ("u2", "Bob", 30),
                ("u3", "Charlie", 25),
            ],
        ).await;

        backend.register_map("users").await.unwrap();
        let batches = backend
            .execute_sql("SELECT age, COUNT(*) as cnt FROM users GROUP BY age ORDER BY age")
            .await
            .unwrap();

        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2); // Two distinct ages: 25 and 30.
    }

    #[tokio::test]
    async fn select_with_order_by() {
        let (backend, factory) = setup_backend();
        populate_users(
            &factory,
            vec![("u1", "Charlie", 35), ("u2", "Alice", 25), ("u3", "Bob", 30)],
        ).await;

        backend.register_map("users").await.unwrap();
        let batches = backend
            .execute_sql("SELECT name, age FROM users ORDER BY name")
            .await
            .unwrap();

        assert!(!batches.is_empty());
        let names = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "Alice");
        assert_eq!(names.value(1), "Bob");
        assert_eq!(names.value(2), "Charlie");
    }

    #[tokio::test]
    async fn register_map_without_schema_returns_error() {
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let sp = Arc::new(TestSchemaProvider::new());
        // Do NOT register a schema for "unknown_map".
        let cache = Arc::new(ArrowCacheManager::new());
        let backend = DataFusionBackend::new(factory, sp, cache);

        let result = backend.register_map("unknown_map").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            QueryBackendError::SchemaRequired(name) => assert_eq!(name, "unknown_map"),
            other => panic!("expected SchemaRequired, got: {other}"),
        }
    }

    #[tokio::test]
    async fn deregister_map_succeeds() {
        let (backend, _factory) = setup_backend();
        backend.register_map("users").await.unwrap();
        backend.deregister_map("users").await.unwrap();

        // Querying after deregister should fail.
        let result = backend.execute_sql("SELECT * FROM users").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_query_delegates_to_predicate_engine() {
        let (backend, _factory) = setup_backend();
        let entries = vec![
            (
                "k1".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("age".into()),
                        rmpv::Value::Integer(30.into()),
                    ),
                ]),
            ),
            (
                "k2".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("age".into()),
                        rmpv::Value::Integer(20.into()),
                    ),
                ]),
            ),
        ];
        let query = Query {
            predicate: Some(topgun_core::messages::PredicateNode {
                op: topgun_core::messages::PredicateOp::Gt,
                attribute: Some("age".to_string()),
                value: Some(rmpv::Value::Integer(25.into())),
                children: None,
            }),
            r#where: None,
            sort: None,
            limit: None,
            cursor: None,
        };

        let result = backend.execute_query("users", entries, &query).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "k1");
    }

    #[tokio::test]
    async fn invalid_sql_returns_error() {
        let (backend, _factory) = setup_backend();
        let result = backend.execute_sql("NOT VALID SQL !@#$").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cache_invalidation_updates_query_results() {
        let (backend, factory) = setup_backend();
        populate_users(&factory, vec![("u1", "Alice", 30)]).await;

        backend.register_map("users").await.unwrap();

        // First query.
        let batches = backend.execute_sql("SELECT * FROM users").await.unwrap();
        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 1);

        // Add another user (this goes directly to the store, simulating a mutation).
        let store = factory.get_or_create("users", 0);
        store.put(
            "u2",
            RecordValue::Lww {
                value: make_user("Bob", 25),
                timestamp: test_timestamp(),
            },
            ExpiryPolicy::NONE,
            CallerProvenance::Client,
        ).await.unwrap();

        // Manually invalidate cache (normally done by ArrowCacheObserver).
        backend.cache_manager.invalidate("users", 0);

        // Re-query should now return 2 rows.
        let batches = backend.execute_sql("SELECT * FROM users").await.unwrap();
        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 2);
    }
}
