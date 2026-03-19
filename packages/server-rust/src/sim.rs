//! Simulation I/O seams for deterministic testing via madsim.
//!
//! This module is only compiled when the `simulation` feature is active.
//! It provides re-exports of madsim's virtual time and seeded RNG so that
//! simulation test code imports from one location rather than depending on
//! madsim directly.
//!
//! # Tokio shim strategy (Option A)
//!
//! madsim provides a `madsim::tokio` re-export that mirrors the tokio API
//! surface. Rather than patching the global `tokio` crate (which `[patch]`
//! sections apply unconditionally and cannot be gated on a Cargo feature),
//! simulation code uses `madsim::tokio` via a cfg-gated import alias:
//!
//! ```rust,ignore
//! #[cfg(feature = "simulation")]
//! use madsim::tokio as tokio;
//! #[cfg(not(feature = "simulation"))]
//! use tokio;
//! ```
//!
//! This keeps the real tokio intact for all non-simulation builds.
//!
//! # I/O boundary summary
//!
//! | I/O Boundary | Normal path          | Simulation path               |
//! |---|---|---|
//! | Async runtime | `tokio`              | `madsim::tokio` (alias above) |
//! | Time          | `tokio::time`        | `sim::time` (this module)     |
//! | RNG           | `rand::thread_rng()` | `sim::rand` (this module)     |
//! | Disk I/O      | in-memory store      | unchanged                     |

/// Virtual-time primitives from madsim.
///
/// Replaces `tokio::time` in simulation builds. All `sleep`, `interval`, and
/// `timeout` calls made through this re-export are controlled by the simulator
/// clock, enabling deterministic time advancement.
pub use madsim::time;

/// Seeded, deterministic RNG from madsim.
///
/// Replaces `rand::thread_rng()` in simulation builds. The simulator seeds
/// this RNG from the test's seed value, so every test run with the same seed
/// produces the same sequence of random numbers.
pub use madsim::rand;
