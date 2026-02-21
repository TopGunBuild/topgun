//! `RecordStore` implementations.
//!
//! Provides concrete implementations of the
//! [`RecordStore`](super::RecordStore) trait.

mod default_record_store;

pub use default_record_store::DefaultRecordStore;
pub use default_record_store::StorageConfig;
