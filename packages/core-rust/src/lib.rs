//! `TopGun` Core — CRDTs, Hybrid Logical Clock, `MerkleTree`, and message schemas.

pub mod context;
pub mod hash;
pub mod hlc;
pub mod merkle;
pub mod schema;
pub mod traits;
pub mod types;

pub use context::RequestContext;
pub use schema::{FieldDef, MapSchema, Predicate, SyncShape, ValidationResult};
pub use traits::{Inbox, Processor, ProcessorContext, QueryNotifier};
pub use types::{CrdtMap, MapType, Principal, StorageValue, Value};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        // Empty body: if this test runs, the crate compiles and loads.
    }
}
