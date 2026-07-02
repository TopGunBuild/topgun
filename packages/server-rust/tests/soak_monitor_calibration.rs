//! CI gate for the soak memory monitor's calibrated thresholds.
//!
//! The soak bench (`benches/soak_harness/`) is declared `harness = false`, so the
//! `#[cfg(test)] mod tests` inside `monitor.rs` never runs under `cargo test
//! --bench soak_harness` (that command runs the bench's `main`, not libtest).
//! Re-including the module here as an integration target puts those calibration
//! tests under the standard libtest harness, so they run as a real gate on
//! `cargo test` / `cargo test --all-targets` (the CI test job).
//!
//! `monitor.rs` is self-contained (only `std`), so including it by path pulls in
//! no other bench state. `allow(dead_code)` covers `sample_rss_mb` and any
//! assessment fields the calibration tests don't read — they are live in the
//! bench binary, just unreferenced from this test crate's view.

#[path = "../benches/soak_harness/monitor.rs"]
#[allow(dead_code)]
mod monitor;
