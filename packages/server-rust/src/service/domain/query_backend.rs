//! Query backend abstraction layer.
//!
//! Defines the `QueryBackend` trait for predicate-based query execution and
//! the feature-gated `SqlQueryBackend` trait for SQL query execution via
//! `DataFusion`. The default `PredicateBackend` delegates to the existing
//! predicate engine.

use std::sync::Arc;

use async_trait::async_trait;
use topgun_core::messages::{Query, QueryResultEntry};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by query backend operations.
#[derive(Debug)]
pub enum QueryBackendError {
    /// SQL parsing or syntax error.
    SqlParse(String),
    /// Execution error during query processing.
    Execution(String),
    /// Map requires a schema for SQL queries but none is registered.
    SchemaRequired(String),
    /// Internal error wrapping anyhow.
    Internal(anyhow::Error),
}

impl std::fmt::Display for QueryBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SqlParse(msg) => write!(f, "SQL parse error: {msg}"),
            Self::Execution(msg) => write!(f, "execution error: {msg}"),
            Self::SchemaRequired(map) => {
                write!(f, "schema required for map '{map}' to execute SQL queries")
            }
            Self::Internal(err) => write!(f, "internal error: {err}"),
        }
    }
}

impl std::error::Error for QueryBackendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Internal(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// QueryBackend trait (always available)
// ---------------------------------------------------------------------------

/// Abstraction over query execution backends.
///
/// The base trait supports predicate-based queries and is always compiled.
/// See `SqlQueryBackend` (feature-gated behind `datafusion`) for SQL support.
#[async_trait]
pub trait QueryBackend: Send + Sync {
    /// Execute a predicate-based query against the provided entries.
    ///
    /// `map_name` is provided for backends that need table context (e.g.,
    /// `DataFusionBackend`). The default `PredicateBackend` ignores it.
    async fn execute_query(
        &self,
        map_name: &str,
        entries: Vec<(String, rmpv::Value)>,
        query: &Query,
    ) -> Result<Vec<QueryResultEntry>, QueryBackendError>;

    /// Register a map as a queryable table.
    async fn register_map(&self, map_name: &str) -> Result<(), QueryBackendError>;

    /// Deregister a map (on map destroy).
    async fn deregister_map(&self, map_name: &str) -> Result<(), QueryBackendError>;
}

// ---------------------------------------------------------------------------
// SqlQueryBackend trait (feature-gated)
// ---------------------------------------------------------------------------

/// Extended query backend with SQL support via `DataFusion`.
///
/// Only available when the `datafusion` feature is enabled.
#[cfg(feature = "datafusion")]
#[async_trait]
pub trait SqlQueryBackend: QueryBackend {
    /// Execute a SQL query string, returning Arrow `RecordBatches`.
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<arrow::array::RecordBatch>, QueryBackendError>;
}

// ---------------------------------------------------------------------------
// PredicateBackend (default backend)
// ---------------------------------------------------------------------------

/// Default query backend that delegates to the existing predicate engine.
///
/// Does not support SQL queries (`SqlQueryBackend` is not implemented).
/// `register_map` and `deregister_map` are no-ops since the predicate engine
/// operates purely on the provided entries slice.
pub struct PredicateBackend;

#[async_trait]
impl QueryBackend for PredicateBackend {
    async fn execute_query(
        &self,
        _map_name: &str,
        entries: Vec<(String, rmpv::Value)>,
        query: &Query,
    ) -> Result<Vec<QueryResultEntry>, QueryBackendError> {
        Ok(super::predicate::execute_query(entries, query))
    }

    async fn register_map(&self, _map_name: &str) -> Result<(), QueryBackendError> {
        Ok(())
    }

    async fn deregister_map(&self, _map_name: &str) -> Result<(), QueryBackendError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience factory functions
// ---------------------------------------------------------------------------

/// Creates a default `PredicateBackend` wrapped in `Arc`.
///
/// Convenience for server assembly call sites that need to pass a
/// `QueryBackend` without SQL support.
#[must_use]
pub fn create_default_backend() -> Arc<PredicateBackend> {
    Arc::new(PredicateBackend)
}

/// Creates a `DataFusionBackend` with the given dependencies.
///
/// Convenience for server assembly when the `datafusion` feature is enabled.
#[cfg(feature = "datafusion")]
#[must_use]
pub fn create_datafusion_backend(
    record_store_factory: Arc<crate::storage::RecordStoreFactory>,
    schema_provider: Arc<dyn crate::traits::SchemaProvider>,
    cache_manager: Arc<crate::service::domain::arrow_cache::ArrowCacheManager>,
) -> Arc<crate::service::domain::datafusion_backend::DataFusionBackend> {
    Arc::new(crate::service::domain::datafusion_backend::DataFusionBackend::new(
        record_store_factory,
        schema_provider,
        cache_manager,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use topgun_core::messages::{PredicateNode, PredicateOp, SortDirection};
    use std::collections::HashMap;

    #[tokio::test]
    async fn predicate_backend_execute_query_filters() {
        let backend = PredicateBackend;
        let entries = vec![
            (
                "user-1".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("age".into()),
                        rmpv::Value::Integer(25.into()),
                    ),
                ]),
            ),
            (
                "user-2".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("age".into()),
                        rmpv::Value::Integer(15.into()),
                    ),
                ]),
            ),
        ];
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Gt,
                attribute: Some("age".to_string()),
                value: Some(rmpv::Value::Integer(18.into())),
                ..Default::default()
            }),
            r#where: None,
            sort: None,
            limit: None,
            cursor: None,
            group_by: None,
        };
        let result = backend
            .execute_query("users", entries, &query)
            .await
            .expect("execute_query should succeed");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "user-1");
    }

    #[tokio::test]
    async fn predicate_backend_execute_query_with_where_clause() {
        let backend = PredicateBackend;
        let entries = vec![
            (
                "k1".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("status".into()),
                        rmpv::Value::String("active".into()),
                    ),
                ]),
            ),
            (
                "k2".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("status".into()),
                        rmpv::Value::String("inactive".into()),
                    ),
                ]),
            ),
        ];
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "status".to_string(),
            rmpv::Value::String("active".into()),
        );
        let query = Query {
            predicate: None,
            r#where: Some(where_clause),
            sort: None,
            limit: None,
            cursor: None,
            group_by: None,
        };
        let result = backend
            .execute_query("items", entries, &query)
            .await
            .expect("execute_query should succeed");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "k1");
    }

    #[tokio::test]
    async fn predicate_backend_execute_query_with_sort_and_limit() {
        let backend = PredicateBackend;
        let entries = vec![
            (
                "a".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("name".into()),
                        rmpv::Value::String("Charlie".into()),
                    ),
                ]),
            ),
            (
                "b".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("name".into()),
                        rmpv::Value::String("Alice".into()),
                    ),
                ]),
            ),
            (
                "c".to_string(),
                rmpv::Value::Map(vec![
                    (
                        rmpv::Value::String("name".into()),
                        rmpv::Value::String("Bob".into()),
                    ),
                ]),
            ),
        ];
        let mut sort = HashMap::new();
        sort.insert("name".to_string(), SortDirection::Asc);
        let query = Query {
            predicate: None,
            r#where: None,
            sort: Some(sort),
            limit: Some(2),
            cursor: None,
            group_by: None,
        };
        let result = backend
            .execute_query("people", entries, &query)
            .await
            .expect("execute_query should succeed");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].key, "b"); // Alice
        assert_eq!(result[1].key, "c"); // Bob
    }

    #[tokio::test]
    async fn predicate_backend_register_deregister_are_noops() {
        let backend = PredicateBackend;
        backend.register_map("test").await.expect("register should succeed");
        backend.deregister_map("test").await.expect("deregister should succeed");
    }

    #[test]
    fn query_backend_error_display() {
        assert_eq!(
            QueryBackendError::SqlParse("bad sql".into()).to_string(),
            "SQL parse error: bad sql"
        );
        assert_eq!(
            QueryBackendError::Execution("timeout".into()).to_string(),
            "execution error: timeout"
        );
        assert_eq!(
            QueryBackendError::SchemaRequired("users".into()).to_string(),
            "schema required for map 'users' to execute SQL queries"
        );
        let err = QueryBackendError::Internal(anyhow::anyhow!("boom"));
        assert_eq!(err.to_string(), "internal error: boom");
    }

    #[test]
    fn query_backend_error_is_error_trait() {
        let err = QueryBackendError::Internal(anyhow::anyhow!("boom"));
        assert!(std::error::Error::source(&err).is_some());

        let err = QueryBackendError::SqlParse("x".into());
        assert!(std::error::Error::source(&err).is_none());
    }
}
