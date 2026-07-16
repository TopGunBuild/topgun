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

/// Storage-layer crash-safety property tests (no-acked-loss invariant).
#[cfg(test)]
mod crash_safety_proptest;

/// Behavioral end-to-end tests for cost-based eviction (AC2–AC4).
#[cfg(test)]
mod eviction_cost_test;

/// Differential resident-state equivalence proof for the OR-Map in-place mutate
/// write path vs the historical get→build→put path.
#[cfg(test)]
mod or_inplace_mutate_proptest;

pub mod datastores;
pub mod durable_merkle;
pub mod engine;
pub mod engines;
pub mod eviction_config;
pub mod eviction_orchestrator;
pub mod factory;
pub mod impls;
pub mod map_data_store;
pub mod merkle_sync;
pub mod mutation_observer;
pub mod record;
pub mod record_store;
pub mod wal;

pub use datastores::*;
pub use durable_merkle::*;
pub use engine::*;
pub use engines::*;
pub use eviction_config::*;
pub use factory::*;
pub use impls::*;
pub use map_data_store::*;
pub use merkle_sync::*;
pub use mutation_observer::*;
pub use record::*;
pub use record_store::*;
