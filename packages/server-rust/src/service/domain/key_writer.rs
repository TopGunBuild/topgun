//! Re-export of the per-key writer registry, which is owned by the storage
//! layer ([`crate::storage::key_writer`]). Kept so that every
//! `service::domain::key_writer` path that existed before the registry moved
//! still resolves.

pub use crate::storage::key_writer::*;
