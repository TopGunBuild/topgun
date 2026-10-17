//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Includes [`NullDataStore`] for testing and ephemeral data,
//! and [`RedbDataStore`] for embedded zero-config persistence (default).

pub mod engine_store;
// The main tests are frozen once committed, so the style lints they trip are
// allowed here, by name, instead of edited away in the file.
#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::needless_pass_by_value,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::type_complexity
)]
mod engine_store_tests;
#[cfg(test)]
mod kv_conformance;
pub mod kv_engine;
mod null;
#[cfg(feature = "redb")]
mod redb;
#[cfg(feature = "redb")]
mod redb_engine;
mod write_behind;

pub use engine_store::EngineMapDataStore;
pub use null::NullDataStore;
#[cfg(feature = "redb")]
pub use redb::RedbDataStore;
#[cfg(feature = "redb")]
pub use redb_engine::RedbKvEngine;
pub use write_behind::{WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
