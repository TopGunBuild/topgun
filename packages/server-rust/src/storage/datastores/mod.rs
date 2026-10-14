//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Includes [`NullDataStore`] for testing and ephemeral data,
//! and [`RedbDataStore`] for embedded zero-config persistence (default).

mod null;
#[cfg(feature = "redb")]
mod redb;
mod write_behind;

pub use null::NullDataStore;
#[cfg(feature = "redb")]
pub use redb::RedbDataStore;
pub use write_behind::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
