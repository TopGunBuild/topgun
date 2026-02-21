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

pub mod engine;
pub mod map_data_store;
pub mod mutation_observer;
pub mod record;
pub mod record_store;

pub use engine::*;
pub use map_data_store::*;
pub use mutation_observer::*;
pub use record::*;
pub use record_store::*;
