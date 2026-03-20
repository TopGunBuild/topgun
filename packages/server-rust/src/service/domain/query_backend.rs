//! Query backend abstraction layer.
//!
//! Defines the `QueryBackend` trait for predicate-based query execution and
//! the feature-gated `SqlQueryBackend` trait for SQL query execution via
//! DataFusion. The default `PredicateBackend` delegates to the existing
//! predicate engine.

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

/// Extended query backend with SQL support via DataFusion.
///
/// Only available when the `datafusion` feature is enabled.
#[cfg(feature = "datafusion")]
#[async_trait]
pub trait SqlQueryBackend: QueryBackend {
    /// Execute a SQL query string, returning Arrow RecordBatches.
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<arrow::array::RecordBatch>, QueryBackendError>;
}

// ---------------------------------------------------------------------------
// PredicateBackend (default backend)
// ---------------------------------------------------------------------------

/// Default query backend that delegates to the predicate engine.
///
/// Does not support SQL queries (`SqlQueryBackend` is not implemented).
pub struct PredicateBackend;
