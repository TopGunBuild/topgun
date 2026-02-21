//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Currently includes [`NullDataStore`] for testing and ephemeral data.

mod null;

pub use null::NullDataStore;
