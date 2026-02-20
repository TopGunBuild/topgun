//! `TopGun` Server — `WebSocket` server with clustering, partitioning, and `PostgreSQL` storage.

pub mod network;
pub mod traits;

pub use traits::{MapProvider, SchemaProvider, ServerStorage};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        // Empty body: if this test runs, the crate compiles and loads.
    }
}
