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
use crate::service::dispatch::PartitionDispatcher;
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

#[async_trait::async_trait]
impl ExpiryDispatch for PartitionDispatcher {
    async fn dispatch(&self, op: Operation) -> Result<OperationResponse, OperationError> {
        PartitionDispatcher::dispatch(self, op).await
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use topgun_core::messages::Message;
    use topgun_core::{ClockSource, Principal, SystemClock, HLC};

    use super::*;
    use crate::network::connection::ConnectionRegistry;
    use crate::service::classify::OperationService;
    use crate::service::dispatch::DispatchConfig;
    use crate::service::domain::crdt::CrdtService;
    use crate::service::domain::query::QueryRegistry;
    use crate::service::domain::schema::SchemaService;
    use crate::service::middleware::pipeline::build_operation_pipeline;
    use crate::service::policy::{
        InMemoryPolicyStore, PermissionAction, PermissionPolicy, PolicyEffect, PolicyEvaluator,
        PolicyStore,
    };
    use crate::service::router::OperationRouter;
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::storage::datastores::NullDataStore;
    use crate::storage::expiry::ManualClock;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::record::RecordValue;

    const DAY: u64 = 86_400_000;
    const MAP: &str = "acme.docs";
    const KEY: &str = "k";

    fn hlc() -> Arc<Mutex<HLC>> {
        Arc::new(Mutex::new(HLC::new(
            "test-node".to_string(),
            Box::new(SystemClock),
        )))
    }

    /// Every policy the store holds denies: everything, and the deny policy a
    /// tenant delete leaves for `acme`.
    async fn deny_everything() -> Arc<PolicyEvaluator> {
        let store = Arc::new(InMemoryPolicyStore::new());
        for (id, pattern) in [("deny-all", "**"), ("tenant-deleted:acme", "acme.**")] {
            store
                .upsert_policy(PermissionPolicy {
                    id: id.to_string(),
                    map_pattern: pattern.to_string(),
                    action: PermissionAction::All,
                    effect: PolicyEffect::Deny,
                    condition: None,
                })
                .await
                .expect("upsert policy");
        }
        Arc::new(PolicyEvaluator::new(store))
    }

    fn put(origin: CallerOrigin, stamp: &Timestamp, ttl_ms: Option<u64>) -> Operation {
        let mut ctx = OperationContext::new(1, service_names::CRDT, stamp.clone(), 5_000);
        ctx.partition_id = Some(hash_to_partition(KEY));
        ctx.caller_origin = origin;
        if origin == CallerOrigin::Client {
            ctx.principal = Some(Principal {
                id: "user-a".to_string(),
                roles: vec![],
                tenant: Some("acme".to_string()),
            });
        }
        Operation::ClientOp {
            ctx,
            payload: topgun_core::messages::ClientOpMessage {
                payload: topgun_core::messages::base::ClientOp {
                    id: Some("put".to_string()),
                    map_name: MAP.to_string(),
                    key: KEY.to_string(),
                    op_type: None,
                    record: Some(Some(topgun_core::LWWRecord {
                        value: Some(rmpv::Value::from("v")),
                        timestamp: stamp.clone(),
                        ttl_ms,
                    })),
                    or_record: None,
                    or_tag: None,
                    write_concern: None,
                    timeout: None,
                },
            },
        }
    }

    #[test]
    fn expire_record_op_targets_the_key_partition_of_the_crdt_service_as_system() {
        let op = expire_record_op(MAP, KEY, ExpiryExpectation::Or);
        let ctx = op.ctx();
        assert_eq!(ctx.partition_id, Some(hash_to_partition(KEY)));
        assert_eq!(ctx.service_name, service_names::CRDT);
        assert_eq!(ctx.caller_origin, CallerOrigin::System);
        assert!(ctx.principal.is_none());
        assert!(ctx.connection_id.is_none());
    }

    /// An `ExpireRecord` sent through `ExpiryDispatch` on a real partition
    /// dispatcher, whose pipeline carries the authorization layer over a store
    /// that denies every map and holds a tenant's deny policy, reaches the CRDT
    /// service and removes the expired record; a client write of the same map
    /// through the same dispatcher is refused, so the layer is in force.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expire_record_reaches_the_handler_through_the_dispatcher_and_authorization() {
        let wall = SystemClock.now();
        let clock = Arc::new(ManualClock::new(wall + 30 * DAY));
        let factory = Arc::new(
            RecordStoreFactory::new(
                StorageConfig::default(),
                Arc::new(NullDataStore),
                Vec::new(),
            )
            .with_clock(Arc::clone(&clock) as Arc<dyn ClockSource>),
        );
        let crdt = Arc::new(CrdtService::new(
            Arc::clone(&factory),
            Arc::new(ConnectionRegistry::new()),
            Arc::new(WriteAdmission::new(
                Arc::new(SecurityConfig::default()),
                hlc(),
            )),
            Arc::new(QueryRegistry::new()),
            Arc::new(SchemaService::new()),
        ));
        let evaluator = deny_everything().await;
        let dispatcher = PartitionDispatcher::new(
            &DispatchConfig {
                worker_count: 2,
                channel_buffer_size: 16,
            },
            || {
                let mut router = OperationRouter::new();
                router.register(service_names::CRDT, Arc::clone(&crdt));
                build_operation_pipeline(
                    router,
                    &ServerConfig::default(),
                    Some(Arc::clone(&evaluator)),
                )
            },
        );
        let stamp = Timestamp {
            millis: wall,
            counter: 1,
            node_id: "writer".to_string(),
        };

        let refused = PartitionDispatcher::dispatch(
            &dispatcher,
            put(CallerOrigin::Client, &stamp, Some(DAY)),
        )
        .await;
        assert!(
            matches!(refused, Err(OperationError::Forbidden { .. })),
            "the authorization layer refuses a client write: {refused:?}"
        );
        PartitionDispatcher::dispatch(&dispatcher, put(CallerOrigin::System, &stamp, Some(DAY)))
            .await
            .expect("a trusted write is applied");
        let store = factory.get_or_create(MAP, hash_to_partition(KEY));
        assert!(matches!(
            store.get(KEY, false).await.unwrap().map(|r| r.value),
            Some(RecordValue::Lww { .. })
        ));

        let via: &dyn ExpiryDispatch = &dispatcher;
        let resp = via
            .dispatch(expire_record_op(
                MAP,
                KEY,
                ExpiryExpectation::Lww {
                    timestamp: stamp.clone(),
                },
            ))
            .await
            .expect("the expire passes authorization");

        assert!(matches!(resp, OperationResponse::Empty), "{resp:?}");
        assert!(
            store.get(KEY, false).await.unwrap().is_none(),
            "the expired record is removed"
        );
    }

    /// `classify` builds no `ExpireRecord` from any message: checked against
    /// every message of the cross-language fixture corpus, the encodings the
    /// TypeScript client produces, under a client and a trusted origin. The
    /// corpus also holds HTTP sync bodies, which are not messages and are
    /// passed over.
    #[test]
    fn classify_never_yields_expire_record() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../core-rust/tests/fixtures");
        let service = OperationService::new(hlc(), Arc::new(ServerConfig::default()));
        let mut messages = 0;
        let mut classified = 0;
        for entry in std::fs::read_dir(&dir).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("msgpack") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("fixture bytes");
            let Ok(msg) = rmp_serde::from_slice::<Message>(&bytes) else {
                continue;
            };
            messages += 1;
            for origin in [CallerOrigin::Client, CallerOrigin::System] {
                if let Ok(op) = service.classify(msg.clone(), None, origin) {
                    classified += 1;
                    assert!(
                        !matches!(op, Operation::ExpireRecord { .. }),
                        "{} classified to ExpireRecord",
                        path.display()
                    );
                }
            }
        }
        assert!(messages >= 60, "the corpus is there: {messages} messages");
        assert!(classified > 0, "some fixtures classify");
    }
}
