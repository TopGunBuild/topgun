#![cfg(feature = "simulation")]

mod cluster_membership;
mod crdt_convergence;
mod event_journal;
mod merkle_sync;
mod proptest_sim;

#[cfg(not(feature = "simulation"))]
fn main() {}
