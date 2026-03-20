#![cfg(feature = "simulation")]

mod crdt_convergence;
mod merkle_sync;
mod cluster_membership;

#[cfg(not(feature = "simulation"))]
fn main() {}
