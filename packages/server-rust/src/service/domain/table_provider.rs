//! DataFusion `TableProvider` implementation for TopGun maps.
//!
//! `TopGunTableProvider` exposes a TopGun map as a DataFusion table,
//! scanning all partitions and converting records to Arrow via the
//! `ArrowCacheManager`. `TopGunExec` implements `ExecutionPlan` as a
//! leaf scan node.
//!
//! All types in this module are feature-gated behind `#[cfg(feature = "datafusion")]`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use arrow::array::RecordBatch;
use arrow::compute::concat_batches;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use datafusion::catalog::Session;
use datafusion::common::Result as DfResult;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::Expr;
use datafusion::physical_expr::EquivalenceProperties;
use datafusion::physical_plan::execution_plan::{Boundedness, EmissionType};
use datafusion::physical_plan::memory::MemoryStream;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
};

use super::arrow_cache::ArrowCacheManager;
use super::arrow_convert::build_record_batch;
use crate::storage::record::RecordValue;
use crate::storage::RecordStoreFactory;

// ---------------------------------------------------------------------------
// TopGunTableProvider
// ---------------------------------------------------------------------------

/// DataFusion `TableProvider` backed by a TopGun map's `RecordStore`.
///
/// On scan, iterates all partitions for the map, builds Arrow batches via
/// `ArrowCacheManager` (lazy caching per partition), concatenates them, and
/// returns a single DataFusion partition.
#[derive(Debug)]
pub struct TopGunTableProvider {
    map_name: String,
    arrow_schema: SchemaRef,
    record_store_factory: Arc<RecordStoreFactory>,
    cache_manager: Arc<ArrowCacheManager>,
}

impl TopGunTableProvider {
    /// Creates a new table provider for the given map.
    #[must_use]
    pub fn new(
        map_name: String,
        arrow_schema: SchemaRef,
        record_store_factory: Arc<RecordStoreFactory>,
        cache_manager: Arc<ArrowCacheManager>,
    ) -> Self {
        Self {
            map_name,
            arrow_schema,
            record_store_factory,
            cache_manager,
        }
    }
}

#[async_trait]
impl TableProvider for TopGunTableProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.arrow_schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(TopGunExec::new(
            self.map_name.clone(),
            Arc::clone(&self.arrow_schema),
            Arc::clone(&self.record_store_factory),
            Arc::clone(&self.cache_manager),
            projection.cloned(),
        )))
    }
}

// ---------------------------------------------------------------------------
// TopGunExec
// ---------------------------------------------------------------------------

/// Leaf `ExecutionPlan` that scans a TopGun map's partitions.
///
/// Aggregates all TopGun partitions into a single DataFusion partition.
/// Uses `ArrowCacheManager` for per-partition caching of Arrow batches.
#[derive(Debug, Clone)]
pub struct TopGunExec {
    map_name: String,
    full_schema: SchemaRef,
    projected_schema: SchemaRef,
    record_store_factory: Arc<RecordStoreFactory>,
    cache_manager: Arc<ArrowCacheManager>,
    projection: Option<Vec<usize>>,
    properties: PlanProperties,
}

impl TopGunExec {
    /// Creates a new execution plan for the given map.
    #[must_use]
    pub fn new(
        map_name: String,
        full_schema: SchemaRef,
        record_store_factory: Arc<RecordStoreFactory>,
        cache_manager: Arc<ArrowCacheManager>,
        projection: Option<Vec<usize>>,
    ) -> Self {
        let projected_schema = if let Some(ref proj) = projection {
            Arc::new(full_schema.project(proj).expect("valid projection"))
        } else {
            Arc::clone(&full_schema)
        };

        let properties = PlanProperties::new(
            EquivalenceProperties::new(Arc::clone(&projected_schema)),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        );

        Self {
            map_name,
            full_schema,
            projected_schema,
            record_store_factory,
            cache_manager,
            projection,
            properties,
        }
    }
}

impl DisplayAs for TopGunExec {
    fn fmt_as(&self, t: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match t {
            DisplayFormatType::Default | DisplayFormatType::Verbose => {
                write!(f, "TopGunExec: map_name={}", self.map_name)?;
                if let Some(ref proj) = self.projection {
                    write!(f, ", projection={proj:?}")?;
                }
                Ok(())
            }
        }
    }
}

impl ExecutionPlan for TopGunExec {
    fn name(&self) -> &str {
        "TopGunExec"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.projected_schema)
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![] // Leaf node.
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        if children.is_empty() {
            Ok(self)
        } else {
            Err(datafusion::error::DataFusionError::Plan(
                "TopGunExec is a leaf node and cannot have children".to_string(),
            ))
        }
    }

    fn properties(&self) -> &PlanProperties {
        &self.properties
    }

    fn execute(
        &self,
        _partition: usize,
        _context: Arc<TaskContext>,
    ) -> DfResult<SendableRecordBatchStream> {
        let stores = self
            .record_store_factory
            .get_all_for_map(&self.map_name);

        let full_schema = Arc::clone(&self.full_schema);

        if stores.is_empty() {
            // No data: return an empty batch with the correct schema.
            let empty = RecordBatch::new_empty(Arc::clone(&self.projected_schema));
            return Ok(Box::pin(MemoryStream::try_new(
                vec![empty],
                Arc::clone(&self.projected_schema),
                None,
            )?));
        }

        // Build or retrieve cached batch for each partition.
        let mut batches = Vec::with_capacity(stores.len());
        for store in &stores {
            let map_name = self.map_name.clone();
            let partition_id = store.partition_id();
            let schema_for_build = Arc::clone(&full_schema);

            // Clone Arc for the closure.
            let store_clone = Arc::clone(store);
            let batch = self
                .cache_manager
                .get_or_build(&map_name, partition_id, move || {
                    let mut entries: Vec<(String, rmpv::Value)> = Vec::new();
                    store_clone.for_each_boxed(
                        &mut |key: &str, record: &crate::storage::record::Record| {
                            match &record.value {
                                RecordValue::Lww { value, .. } => {
                                    entries.push((
                                        key.to_string(),
                                        crate::service::domain::predicate::value_to_rmpv(value),
                                    ));
                                }
                                RecordValue::OrMap { .. } | RecordValue::OrTombstones { .. } => {
                                    tracing::warn!(
                                        key = key,
                                        "skipping non-LWW record in SQL table scan"
                                    );
                                }
                            }
                        },
                        false,
                    );
                    build_record_batch(&entries, &schema_for_build)
                })
                .map_err(|e| {
                    datafusion::error::DataFusionError::Execution(e.to_string())
                })?;
            batches.push(batch);
        }

        // Concatenate all partition batches.
        let concatenated = if batches.len() == 1 {
            batches.into_iter().next().unwrap()
        } else {
            concat_batches(&full_schema, &batches).map_err(|e| {
                datafusion::error::DataFusionError::ArrowError(e, None)
            })?
        };

        // Apply projection if needed.
        let final_batch = if let Some(ref proj) = self.projection {
            concatenated.project(proj).map_err(|e| {
                datafusion::error::DataFusionError::ArrowError(e, None)
            })?
        } else {
            concatenated
        };

        Ok(Box::pin(MemoryStream::try_new(
            vec![final_batch],
            Arc::clone(&self.projected_schema),
            None,
        )?))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use topgun_core::hlc::Timestamp;
    use topgun_core::types::Value;

    use crate::storage::datastores::NullDataStore;
    use crate::storage::impls::StorageConfig;
    use crate::storage::RecordStoreFactory;
    use crate::storage::{CallerProvenance, ExpiryPolicy};
    use crate::storage::record::{RecordMetadata, RecordValue};

    fn test_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("_key", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("age", DataType::Int64, true),
        ]))
    }

    fn test_timestamp() -> Timestamp {
        Timestamp {
            millis: 1000,
            counter: 0,
            node_id: "test".to_string(),
        }
    }

    async fn populate_store(factory: &RecordStoreFactory, map_name: &str, partition_id: u32, records: Vec<(&str, Value)>) {
        let store = factory.get_or_create(map_name, partition_id);
        for (key, value) in records {
            let record_value = RecordValue::Lww {
                value,
                timestamp: test_timestamp(),
            };
            store.put(key, record_value, ExpiryPolicy::NONE, CallerProvenance::Client).await.unwrap();
        }
    }

    #[test]
    fn table_provider_schema_matches() {
        let schema = test_schema();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let cache = Arc::new(ArrowCacheManager::new());
        let provider = TopGunTableProvider::new(
            "users".to_string(),
            schema.clone(),
            factory,
            cache,
        );

        assert_eq!(provider.schema(), schema);
        assert_eq!(provider.table_type(), TableType::Base);
    }

    #[tokio::test]
    async fn exec_returns_correct_data() {
        let schema = test_schema();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let cache = Arc::new(ArrowCacheManager::new());

        // Populate with data using BTreeMap for the Map value.
        let mut user_map = std::collections::BTreeMap::new();
        user_map.insert("name".to_string(), Value::String("Alice".to_string()));
        user_map.insert("age".to_string(), Value::Int(30));
        populate_store(&factory, "users", 0, vec![("u1", Value::Map(user_map))]).await;

        let exec = TopGunExec::new(
            "users".to_string(),
            schema.clone(),
            Arc::clone(&factory),
            Arc::clone(&cache),
            None,
        );

        let ctx = Arc::new(TaskContext::default());
        let stream = exec.execute(0, ctx).unwrap();

        let batches: Vec<RecordBatch> =
            datafusion::physical_plan::common::collect(stream).await.unwrap();

        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);

        let keys = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(keys.value(0), "u1");

        let names = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "Alice");

        let ages = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(ages.value(0), 30);
    }

    #[tokio::test]
    async fn exec_supports_projection_pushdown() {
        let schema = test_schema();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let cache = Arc::new(ArrowCacheManager::new());

        let mut user_map = std::collections::BTreeMap::new();
        user_map.insert("name".to_string(), Value::String("Bob".to_string()));
        user_map.insert("age".to_string(), Value::Int(25));
        populate_store(&factory, "users", 0, vec![("u1", Value::Map(user_map))]).await;

        // Project only _key (0) and age (2).
        let exec = TopGunExec::new(
            "users".to_string(),
            schema,
            Arc::clone(&factory),
            Arc::clone(&cache),
            Some(vec![0, 2]),
        );

        assert_eq!(exec.schema().fields().len(), 2);
        assert_eq!(exec.schema().field(0).name(), "_key");
        assert_eq!(exec.schema().field(1).name(), "age");

        let ctx = Arc::new(TaskContext::default());
        let stream = exec.execute(0, ctx).unwrap();
        let batches: Vec<RecordBatch> =
            datafusion::physical_plan::common::collect(stream).await.unwrap();

        assert_eq!(batches[0].num_columns(), 2);
        let ages = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(ages.value(0), 25);
    }

    #[tokio::test]
    async fn exec_empty_map_returns_empty_batch() {
        let schema = test_schema();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let cache = Arc::new(ArrowCacheManager::new());

        let exec = TopGunExec::new(
            "empty_map".to_string(),
            schema,
            factory,
            cache,
            None,
        );

        let ctx = Arc::new(TaskContext::default());
        let stream = exec.execute(0, ctx).unwrap();
        let batches: Vec<RecordBatch> =
            datafusion::physical_plan::common::collect(stream).await.unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 0);
    }

    #[tokio::test]
    async fn exec_multiple_partitions_concatenated() {
        let schema = test_schema();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let cache = Arc::new(ArrowCacheManager::new());

        // Populate two partitions.
        let mut m1 = std::collections::BTreeMap::new();
        m1.insert("name".to_string(), Value::String("Alice".to_string()));
        m1.insert("age".to_string(), Value::Int(30));
        populate_store(&factory, "users", 0, vec![("u1", Value::Map(m1))]).await;

        let mut m2 = std::collections::BTreeMap::new();
        m2.insert("name".to_string(), Value::String("Bob".to_string()));
        m2.insert("age".to_string(), Value::Int(25));
        populate_store(&factory, "users", 1, vec![("u2", Value::Map(m2))]).await;

        let exec = TopGunExec::new(
            "users".to_string(),
            schema,
            Arc::clone(&factory),
            Arc::clone(&cache),
            None,
        );

        let ctx = Arc::new(TaskContext::default());
        let stream = exec.execute(0, ctx).unwrap();
        let batches: Vec<RecordBatch> =
            datafusion::physical_plan::common::collect(stream).await.unwrap();

        // Both partitions should be concatenated into one batch.
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);
    }

    #[test]
    fn exec_is_leaf_node() {
        let schema = test_schema();
        let factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        let cache = Arc::new(ArrowCacheManager::new());

        let exec = TopGunExec::new(
            "test".to_string(),
            schema,
            factory,
            cache,
            None,
        );

        assert!(exec.children().is_empty());

        let exec_arc: Arc<dyn ExecutionPlan> = Arc::new(exec);
        // with_new_children with empty vec should succeed.
        let result = exec_arc.clone().with_new_children(vec![]);
        assert!(result.is_ok());

        // with_new_children with non-empty vec should fail.
        let result = exec_arc.with_new_children(vec![Arc::new(TopGunExec::new(
            "other".to_string(),
            test_schema(),
            Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )),
            Arc::new(ArrowCacheManager::new()),
            None,
        ))]);
        assert!(result.is_err());
    }
}
