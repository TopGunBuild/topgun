//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Includes [`NullDataStore`] for testing and ephemeral data,
//! [`RedbDataStore`] for embedded zero-config persistence (default), and
//! [`PostgresDataStore`] for durable production persistence (behind the
//! `postgres` feature).

mod null;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "redb")]
mod redb;
mod write_behind;

pub use null::NullDataStore;
#[cfg(feature = "postgres")]
pub use postgres::PostgresDataStore;
#[cfg(feature = "redb")]
pub use redb::RedbDataStore;
pub use write_behind::{WriteBehindConfig, WriteBehindDataStore};
