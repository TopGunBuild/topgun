//! Removal of expired records through the ordinary operation pipeline
//! (TG-TTL-001).
//!
//! An expired record is removed by an [`Operation::ExpireRecord`], which the
//! CRDT service re-checks under the key's writer and applies as an ordinary
//! REMOVE or `OR_REMOVE`. This module lives under `service/` rather than beside
//! the expiry predicate in `storage/` because it names `service::operation`
//! types, and `storage/` does not depend on `service/`.

use topgun_core::{hash_to_partition, Timestamp};

use crate::service::config::ServerConfig;
use crate::service::operation::{
    service_names, CallerOrigin, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::storage::expiry::ExpiryExpectation;

/// Expiry removals applied, by `kind`: `lww_record` per LWW key, `or_tag` per
/// OR entry.
pub const METRIC_EXPIRY_REMOVED_TOTAL: &str = "topgun_expiry_removed_total";

/// Expiry ops that removed nothing, by `reason`: `absent`, `changed` or
/// `not_expired`.
pub const METRIC_EXPIRY_SKIPPED_TOTAL: &str = "topgun_expiry_skipped_total";

/// Where an expiry removal is sent: the partition dispatcher in production, so
/// it is routed, ordered and back-pressured like any other operation.
#[async_trait::async_trait]
pub trait ExpiryDispatch: Send + Sync {
    /// Sends `op` and waits for its outcome.
    async fn dispatch(&self, op: Operation) -> Result<OperationResponse, OperationError>;
}

/// The removal of `key` of `map_name` that the expiry sweep saw expired as
/// `expected`.
///
/// Routed to the key's partition and the CRDT service, with the `System`
/// origin, which skips authorization: the op is built by the server, never
/// classified from a client message. Its context carries no HLC stamp because
/// the handler reads none; the journal stamp of each removal is minted when it
/// is applied.
#[must_use]
pub fn expire_record_op(map_name: &str, key: &str, expected: ExpiryExpectation) -> Operation {
    let mut ctx = OperationContext::new(
        0,
        service_names::CRDT,
        Timestamp {
            millis: 0,
            counter: 0,
            node_id: String::new(),
        },
        ServerConfig::default().default_operation_timeout_ms,
    );
    ctx.partition_id = Some(hash_to_partition(key));
    ctx.caller_origin = CallerOrigin::System;
    Operation::ExpireRecord {
        ctx,
        map_name: map_name.to_string(),
        key: key.to_string(),
        expected,
    }
}
