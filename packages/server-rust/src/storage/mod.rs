//! Multi-layer storage system for the `TopGun` server.
//!
//! Provides the trait hierarchy and shared types for the three-layer
//! storage architecture:
//!
//! - **Layer 1** ([`StorageEngine`]): Low-level in-memory key-value storage
//! - **Layer 2** ([`RecordStore`]): Per-map-per-partition orchestration with
//!   metadata, expiry, eviction, and mutation observation
//! - **Layer 3** ([`MapDataStore`]): External persistence backend
//!
//! Additionally defines [`MutationObserver`] for reacting to record mutations
//! and [`CompositeMutationObserver`] for fan-out to multiple observers.

pub mod datastores;
pub mod engine;
pub mod engines;
pub mod factory;
pub mod impls;
pub mod map_data_store;
pub mod merkle_sync;
pub mod mutation_observer;
pub mod query_merkle;
pub mod shape_merkle;
pub mod record;
pub mod record_store;

pub use datastores::*;
pub use engine::*;
pub use engines::*;
pub use factory::*;
pub use impls::*;
pub use map_data_store::*;
pub use merkle_sync::*;
pub use mutation_observer::*;
pub use query_merkle::*;
pub use record::*;
pub use record_store::*;
