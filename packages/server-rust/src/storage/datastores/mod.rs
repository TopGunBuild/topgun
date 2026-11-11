//! `MapDataStore` implementations.
//!
//! Provides concrete backends for the [`MapDataStore`](super::MapDataStore) trait.
//! Includes [`NullDataStore`] for testing and ephemeral data,
//! and [`RedbDataStore`] for embedded zero-config persistence (default).

#[cfg(all(feature = "redb", feature = "rocksdb"))]
pub mod engine_migration;
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
// Reachable from the WAL's crash proofs, which need the redb crash backend.
#[cfg(all(test, feature = "redb"))]
pub(crate) mod kv_crash;
// The main tests are frozen once committed; the style lints they trip are
// allowed here, by name, as for the engine store's.
#[allow(clippy::cast_possible_truncation)]
#[cfg(all(test, feature = "redb"))]
mod kv_crash_tests;
pub mod kv_engine;
mod null;
#[cfg(feature = "redb")]
mod redb;
#[cfg(feature = "redb")]
mod redb_engine;
#[cfg(feature = "rocksdb")]
pub mod rocks_engine;
#[cfg(all(feature = "redb", feature = "rocksdb"))]
pub mod rocks_store;
mod write_behind;

pub use engine_store::EngineMapDataStore;
pub use null::NullDataStore;
#[cfg(feature = "redb")]
pub use redb::RedbDataStore;
#[cfg(feature = "redb")]
pub use redb_engine::RedbKvEngine;
pub use write_behind::{BackgroundFailure, WalBootstrap, WriteBehindConfig, WriteBehindDataStore};
