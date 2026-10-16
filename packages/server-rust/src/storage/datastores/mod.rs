//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Includes [`NullDataStore`] for testing and ephemeral data,
//! and [`RedbDataStore`] for embedded zero-config persistence (default).

pub mod kv_engine;
mod null;
#[cfg(feature = "redb")]
mod redb;
#[cfg(feature = "redb")]
mod redb_engine;
mod write_behind;

pub use null::NullDataStore;
#[cfg(feature = "redb")]
pub use redb::RedbDataStore;
#[cfg(feature = "redb")]
pub use redb_engine::RedbKvEngine;
pub use write_behind::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
