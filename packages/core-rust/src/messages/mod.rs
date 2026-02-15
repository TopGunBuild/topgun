//! Wire-compatible message schemas for the TopGun protocol.
//!
//! Each submodule corresponds to a domain of messages defined in the TypeScript
//! Zod schemas (`packages/core/src/schemas/`). All types use named MsgPack
//! serialization (`rmp_serde::to_vec_named()`) with camelCase field names to
//! match the TypeScript wire format.

pub mod base;

// Future submodules (SPEC-052b through SPEC-052e):
// pub mod sync;
// pub mod query;
// pub mod search;
// pub mod cluster;
// pub mod messaging;
// pub mod client_events;
// pub mod http_sync;

pub use base::{
    AuthMessage, AuthRequiredMessage, ChangeEventType, ClientOp, PredicateNode, PredicateOp,
    Query, SortDirection, WriteConcern,
};
