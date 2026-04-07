//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Includes [`NullDataStore`] for testing and ephemeral data, and
//! [`PostgresDataStore`] for durable persistence (behind the `postgres` feature).

mod null;
#[cfg(feature = "postgres")]
mod postgres;
mod write_behind;

pub use null::NullDataStore;
#[cfg(feature = "postgres")]
pub use postgres::PostgresDataStore;
pub use write_behind::{WriteBehindConfig, WriteBehindDataStore};
