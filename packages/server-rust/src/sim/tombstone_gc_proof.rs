//! End-to-end proof of the OR-Map tombstone no-resurrection invariant.
//!
//! The canonical invariant — *a removed value is never resurrected for a known
//! monotone client* — holds IFF BOTH sides are wired: the PRUNE side (a lagging
//! but still-tracked replica pins the epoch so the tombstone is never dropped)
//! AND the RE-ADMISSION side (a forgotten client past retention is gated out of
//! the push path). These tests exercise exactly the "one side alone is
//! insufficient" property.
//!
//! The proof rides `ORMapPushDiff` (the push path preserves the inbound tag
//! verbatim) rather than the op path: an op-path `OR_ADD` is always re-stamped
//! with a fresh server tag, so it is a semantically new add, not a resurrection of a
//! removed tag — the op-path exclusion is sound at the tag level, and a same-tag
//! op-path test would pass vacuously.
//!
//! Living under the `sim` module so `pnpm test:sim` runs the forgotten-client and
//! interleaved-prune (TOCTOU) fault-injection scenarios; the gate/merge handlers
//! are driven directly because the multi-node `SimCluster` harness has no raw
//! `ORMapPushDiff` sender (it only carries op-path writes + Merkle sync).

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tower::ServiceExt;

    use topgun_core::hlc::Timestamp;
    use topgun_core::messages::{
        Message, ORMapEntry, ORMapPushDiff, ORMapPushDiffPayload, ORMapSyncInit,
    };
    use topgun_core::types::Value;
    use topgun_core::{hash_to_partition, ORMapRecord as WireOrRecord};

    use crate::network::config::ConnectionConfig;
    use crate::network::connection::{ConnectionId, ConnectionKind, ConnectionRegistry};
    use crate::network::device_identity::frontier_client_id;
    use crate::service::domain::crdt::prune_epoch_tombstones;
    use crate::service::domain::key_writer::KeyWriterRegistry;
    use crate::service::domain::sync::SyncService;
    use crate::service::operation::{
        service_names, Operation, OperationContext, OperationResponse,
    };
    use crate::storage::merkle_sync::MerkleSyncManager;
    use crate::storage::record::{OrMapEntry, RecordValue};
    use crate::storage::{
        CallerProvenance, ExpiryPolicy, NullDataStore, RecordStoreFactory, StorageConfig,
    };
    use crate::tombstone_frontier::ClientId;
    use crate::tombstone_frontier_impl::TombstoneFrontier;

    // ------------------------------------------------------------------
    // Harness (mirrors the gated SyncService setup used by the SPEC-342c
    // sync-path tests; the `sim` module cannot import their `#[cfg(test)]`
    // private helpers, so it reconstructs the minimal scaffold here and also
    // hands back the shared `KeyWriterRegistry` for the interleaved-prune test).
    // ------------------------------------------------------------------

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 0,
            node_id: "node-1".to_string(),
        }
    }

    fn make_ctx() -> OperationContext {
        OperationContext::new(1, service_names::SYNC, make_timestamp(), 5000)
    }

    fn make_ctx_conn(conn: ConnectionId) -> OperationContext {
        let mut ctx = make_ctx();
        ctx.connection_id = Some(conn);
        ctx
    }

    fn make_factory() -> Arc<RecordStoreFactory> {
        Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ))
    }

    type GatedSync = (
        Arc<SyncService>,
        Arc<RecordStoreFactory>,
        Arc<TombstoneFrontier>,
        Arc<ConnectionRegistry>,
        Arc<KeyWriterRegistry>,
    );

    /// A `SyncService` wired with a frontier + per-key writer + the shared
    /// connection registry (so `resolve_client_id` can find a device-bound
    /// identity). Returns the writer too, for the interleaved-prune sweep.
    fn gated_sync() -> GatedSync {
        let factory = make_factory();
        let frontier = Arc::new(TombstoneFrontier::new(None));
        frontier.set_epoch_width(1); // one epoch per stamp, for precise control
        let key_writer = Arc::new(KeyWriterRegistry::new());
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(
            SyncService::new(
                Arc::new(MerkleSyncManager::default()),
                Arc::clone(&factory),
                Arc::clone(&registry),
            )
            .with_frontier(Arc::clone(&frontier), Arc::clone(&key_writer)),
        );
        (svc, factory, frontier, registry, key_writer)
    }

    /// Register a client connection bound to `device_id` (no principal → the
    /// `NO_AUTH` sentinel), returning its `ConnectionId` and the `ClientId` the
    /// gate derives for it.
    async fn register_device(
        reg: &ConnectionRegistry,
        device_id: &str,
    ) -> (ConnectionId, ClientId) {
        let (handle, _rx) = reg.register(ConnectionKind::Client, &ConnectionConfig::default());
        handle.metadata.write().await.device_id = Some(device_id.to_string());
        let client = frontier_client_id(None, device_id);
        (handle.id, client)
    }

    fn wire_record(tag: &str) -> WireOrRecord<rmpv::Value> {
        WireOrRecord {
            value: rmpv::Value::Integer(1.into()),
            timestamp: make_timestamp(),
            tag: tag.to_string(),
            ttl_ms: None,
        }
    }

    /// A push-diff carrying arbitrary records + tombstones for one key.
    fn push_entry(
        ctx: OperationContext,
        map: &str,
        key: &str,
        records: &[&str],
        tombstones: &[&str],
    ) -> Operation {
        Operation::ORMapPushDiff {
            ctx,
            payload: ORMapPushDiff {
                payload: ORMapPushDiffPayload {
                    map_name: map.to_string(),
                    entries: vec![ORMapEntry {
                        key: key.to_string(),
                        records: records.iter().copied().map(wire_record).collect(),
                        tombstones: tombstones.iter().copied().map(String::from).collect(),
                    }],
                },
            },
        }
    }

    async fn seed_or_map(
        factory: &RecordStoreFactory,
        map: &str,
        key: &str,
        records: &[&str],
        tombstones: &[&str],
    ) {
        let store = factory.get_or_create(map, hash_to_partition(key));
        store
            .put(
                key,
                RecordValue::OrMap {
                    records: records
                        .iter()
                        .map(|t| OrMapEntry {
                            value: Value::Int(1),
                            tag: (*t).to_string(),
                            timestamp: make_timestamp(),
                        })
                        .collect(),
                    tombstones: tombstones.iter().copied().map(String::from).collect(),
                },
                ExpiryPolicy::NONE,
                CallerProvenance::CrdtMerge,
            )
            .await
            .unwrap();
    }

    /// Reads back `(live tags, tombstones)` for a key.
    async fn stored_or_map(
        factory: &RecordStoreFactory,
        map: &str,
        key: &str,
    ) -> (Vec<String>, Vec<String>) {
        let store = factory.get_or_create(map, hash_to_partition(key));
        match store.get(key, false).await.unwrap().map(|r| r.value) {
            Some(RecordValue::OrMap {
                records,
                tombstones,
            }) => (records.into_iter().map(|e| e.tag).collect(), tombstones),
            Some(RecordValue::OrTombstones { tags }) => (Vec::new(), tags),
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// Track a client as an admitted, non-forgotten replica at `cursor`.
    async fn track_client(
        frontier: &TombstoneFrontier,
        client: &ClientId,
        conn: ConnectionId,
        cursor: u64,
    ) {
        frontier.set_delivered(conn, 100);
        assert!(
            frontier.confirm_apply_ack(client, cursor, conn).await,
            "cursor advance should succeed"
        );
    }

    // ==================================================================
    // R8(a) — lagging-but-known replica: prune-safe, merge suppresses the
    // re-pushed removed tag, the rest of the union is admitted normally.
    // ==================================================================

    /// A still-tracked (lagging-but-known) replica re-pushes an already-removed
    /// tag together with a fresh live add. The removed tag stays suppressed by
    /// remove-wins carry-tombstone merge; the live add and the tombstone union
    /// are admitted normally. No resurrection.
    #[tokio::test]
    async fn lagging_known_replica_repush_suppressed_union_admitted() {
        let (svc, factory, frontier, registry, _kw) = gated_sync();
        let (conn, client) = register_device(&registry, "dev-lagging").await;
        let (map, key) = ("omap", "k1");

        // Server already removed R1 (its tombstone is still present — the lagging
        // replica pins the epoch, so it was never pruned).
        seed_or_map(&factory, map, key, &[], &["R1"]).await;
        frontier.stamp_tombstone(map, key, "R1"); // current_epoch = 1
        track_client(&frontier, &client, conn, 1).await;
        frontier.set_durable_epoch_watermark(1000); // protection active
        assert!(
            !frontier.is_forgotten(&client),
            "lagging replica is still known"
        );

        // Re-push the removed R1 (stale) + a genuinely new live add R2, carrying
        // the tombstone union {R1, R9}.
        Arc::clone(&svc)
            .oneshot(push_entry(
                make_ctx_conn(conn),
                map,
                key,
                &["R1", "R2"],
                &["R1", "R9"],
            ))
            .await
            .expect("ack");

        let (live, tombs) = stored_or_map(&factory, map, key).await;
        assert!(
            !live.contains(&"R1".to_string()),
            "removed tag stays suppressed by remove-wins carry-tombstone merge"
        );
        assert!(
            live.contains(&"R2".to_string()),
            "the genuinely new live add is admitted"
        );
        assert!(
            tombs.contains(&"R9".to_string()) && tombs.contains(&"R1".to_string()),
            "pushed tombstone union is admitted normally"
        );
    }

    // ==================================================================
    // R8(b) — the two-path proof: after the tombstone is pruned, only the
    // RE-ADMISSION gate prevents a forgotten client from resurrecting it.
    // ==================================================================

    /// Two-path proof. The server has pruned the tombstone (the client was
    /// forgotten past retention, so nothing pins the epoch). A forgotten client
    /// returns and pushes the stale record:
    ///   • WITHOUT the gate (dark) → the add merges → **resurrection** (the
    ///     documented degradation the gate exists to prevent).
    ///   • WITH the gate armed → the forgotten push is rejected before merge →
    ///     no resurrection.
    #[tokio::test]
    async fn forgotten_client_resurrects_dark_gate_blocks_armed() {
        let (map, key) = ("omap", "k1");

        // --- DARK: protection inactive (watermark 0). ---
        {
            let (svc, factory, frontier, registry, _kw) = gated_sync();
            let (conn, _client) = register_device(&registry, "dev-forgotten").await;
            // Tombstone already pruned → the key is empty; nothing suppresses R1.
            seed_or_map(&factory, map, key, &[], &[]).await;
            frontier.stamp_tombstone(map, "seed", "seed-tag"); // current_epoch = 1, still dark
            Arc::clone(&svc)
                .oneshot(push_entry(make_ctx_conn(conn), map, key, &["R1"], &[]))
                .await
                .expect("ack");
            let (live, _t) = stored_or_map(&factory, map, key).await;
            assert!(
                live.contains(&"R1".to_string()),
                "ABSENT the gate, a forgotten client's stale push resurrects the removed value (documented degradation)"
            );
        }

        // --- ARMED: protection active (watermark > 0). ---
        {
            let (svc, factory, frontier, registry, _kw) = gated_sync();
            let (conn, client) = register_device(&registry, "dev-forgotten").await;
            seed_or_map(&factory, map, key, &[], &[]).await;
            frontier.stamp_tombstone(map, "seed", "seed-tag"); // current_epoch = 1
            frontier.set_durable_epoch_watermark(1000); // protection active
            assert!(
                frontier.is_forgotten(&client),
                "untracked client is forgotten"
            );
            Arc::clone(&svc)
                .oneshot(push_entry(make_ctx_conn(conn), map, key, &["R1"], &[]))
                .await
                .expect("ack");
            let (live, _t) = stored_or_map(&factory, map, key).await;
            assert!(
                !live.contains(&"R1".to_string()),
                "WITH the gate, the forgotten push is rejected before merge — no resurrection"
            );
        }
    }

    // ==================================================================
    // R8(c) — gate-input stability: a gated client's Merkle round-trip must not
    // advance the `delivered(conn)` admission input the push gate reads.
    // ==================================================================

    fn merkle_req(conn: ConnectionId, map: &str) -> Operation {
        use topgun_core::messages::{ORMapMerkleReqBucket, ORMapMerkleReqBucketPayload};
        Operation::ORMapMerkleReqBucket {
            ctx: make_ctx_conn(conn),
            payload: ORMapMerkleReqBucket {
                payload: ORMapMerkleReqBucketPayload {
                    map_name: map.to_string(),
                    path: String::new(),
                },
            },
        }
    }

    /// A forgotten/gated client performs a Merkle round-trip (the handler that
    /// eagerly advances `delivered` for a NON-gated client) and then sends a
    /// stale push. The round-trip must NOT advance `delivered(conn)`, so the
    /// push gate still rejects.
    #[tokio::test]
    async fn merkle_roundtrip_does_not_advance_gated_delivered_push_still_rejected() {
        let (svc, factory, frontier, registry, _kw) = gated_sync();
        let (conn, _client) = register_device(&registry, "dev-forgotten").await;
        let (map, key) = ("omap", "k1");
        seed_or_map(&factory, map, key, &[], &[]).await;
        frontier.stamp_tombstone(map, "seed", "seed-tag"); // current_epoch = 1
        frontier.set_durable_epoch_watermark(1000); // protection active

        assert_eq!(
            frontier.delivered(conn),
            0,
            "gated client starts un-admitted"
        );
        // Merkle round-trip — eager `set_delivered` is suppressed for a gated conn.
        Arc::clone(&svc)
            .oneshot(merkle_req(conn, map))
            .await
            .expect("buckets");
        assert_eq!(
            frontier.delivered(conn),
            0,
            "the gated client's Merkle round-trip must NOT advance its delivered() admission input"
        );

        // The stale push therefore still fails the gate.
        Arc::clone(&svc)
            .oneshot(push_entry(make_ctx_conn(conn), map, key, &["R1"], &[]))
            .await
            .expect("ack");
        let (live, _t) = stored_or_map(&factory, map, key).await;
        assert!(
            !live.contains(&"R1".to_string()),
            "stale push still rejected after the Merkle round-trip"
        );
    }

    /// Reused-connection variant: the SAME socket completes a HEALTHY round
    /// (`delivered > 0`, push admitted), then issues a REGRESSED sync-init. The
    /// gated routing resets `delivered(conn)` to 0; a subsequent Merkle
    /// round-trip does not re-advance it, and a stale push is rejected — the
    /// stale admitted signal cannot be reused.
    #[tokio::test]
    async fn reused_connection_regressed_then_stale_push_rejected() {
        let (svc, factory, frontier, registry, _kw) = gated_sync();
        let (conn, client) = register_device(&registry, "dev-reused").await;
        let map = "omap";
        frontier.stamp_tombstone(map, "seed", "seed-tag"); // current_epoch = 1
        track_client(&frontier, &client, conn, 1).await; // healthy round on THIS conn
        frontier.set_durable_epoch_watermark(1000); // protection active

        // Precondition: while healthy (delivered > 0), a push IS admitted.
        Arc::clone(&svc)
            .oneshot(push_entry(make_ctx_conn(conn), map, "k_pre", &["R0"], &[]))
            .await
            .expect("ack");
        assert!(
            stored_or_map(&factory, map, "k_pre")
                .await
                .0
                .contains(&"R0".to_string()),
            "healthy admitted connection's push is stored (precondition)"
        );

        // Regressed sync-init on the reused socket resets the admitted signal.
        let resp = Arc::clone(&svc)
            .oneshot(Operation::ORMapSyncInit {
                ctx: make_ctx_conn(conn),
                payload: ORMapSyncInit {
                    map_name: map.to_string(),
                    root_hash: 0,
                    bucket_hashes: std::collections::HashMap::new(),
                    last_sync_timestamp: None,
                    claimed_epoch: Some(0), // below the stored cursor → regressed
                },
            })
            .await
            .expect("root");
        assert!(
            matches!(resp, OperationResponse::Message(ref m) if matches!(**m, Message::ORMapSyncRespRoot(_))),
            "regressed replica routed to a resync root"
        );
        assert_eq!(
            frontier.delivered(conn),
            0,
            "regressed sync-init resets the reused connection's delivered() to 0"
        );

        // Merkle round-trip does not re-advance the now-gated delivered input.
        Arc::clone(&svc)
            .oneshot(merkle_req(conn, map))
            .await
            .expect("buckets");
        assert_eq!(
            frontier.delivered(conn),
            0,
            "gated Merkle round-trip does not re-admit"
        );

        // A fresh stale push on the reused socket is now rejected.
        Arc::clone(&svc)
            .oneshot(push_entry(make_ctx_conn(conn), map, "k_post", &["R1"], &[]))
            .await
            .expect("ack");
        assert!(
            !stored_or_map(&factory, map, "k_post")
                .await
                .0
                .contains(&"R1".to_string()),
            "stale push on the reused (now un-admitted) connection is rejected"
        );
    }

    // ==================================================================
    // R8(f) — the Merkle handlers are read-only w.r.t. stored OR value state.
    // ==================================================================

    /// A Merkle round-trip (dark: watermark 0 → no prune) must not mutate the
    /// stored records/tombstones for any key — it is a read-only surface, not a
    /// second inbound admission path.
    #[tokio::test]
    async fn merkle_roundtrip_is_readonly_wrt_or_value_state() {
        let (svc, factory, _frontier, registry, _kw) = gated_sync();
        let (conn, _client) = register_device(&registry, "dev-reader").await;
        let (map, key) = ("omap", "k1");
        seed_or_map(&factory, map, key, &["R1"], &["T1"]).await;

        let before = stored_or_map(&factory, map, key).await;
        Arc::clone(&svc)
            .oneshot(merkle_req(conn, map))
            .await
            .expect("buckets");
        {
            use topgun_core::messages::{ORMapDiffRequest, ORMapDiffRequestPayload};
            Arc::clone(&svc)
                .oneshot(Operation::ORMapDiffRequest {
                    ctx: make_ctx_conn(conn),
                    payload: ORMapDiffRequest {
                        payload: ORMapDiffRequestPayload {
                            map_name: map.to_string(),
                            keys: vec![key.to_string()],
                        },
                    },
                })
                .await
                .expect("diff");
        }
        let after = stored_or_map(&factory, map, key).await;
        assert_eq!(
            before, after,
            "Merkle handlers never mutate stored OR value state"
        );
    }

    // ==================================================================
    // AC6 — TOCTOU: an interleaved prune sweep during the gate→commit window
    // never resurrects. The per-key single writer serializes push-commit and
    // prune, and the gate-time not-forgotten decision is re-checked at commit.
    // ==================================================================

    /// Fault injection: a prune sweep runs concurrently with an admitted push
    /// carrying an already-removed tag. Across every interleaving the removed
    /// tag is never resurrected — the shared per-key writer serializes the
    /// push's gate→commit span against the prune drain.
    #[tokio::test(flavor = "multi_thread")]
    async fn interleaved_prune_during_push_never_resurrects() {
        for round in 0..64u64 {
            let (svc, factory, frontier, registry, key_writer) = gated_sync();
            let (conn, client) = register_device(&registry, "dev-toctou").await;
            let (map, key) = ("omap", "k1");

            // The key already holds a live record R2 and an OLD removed tag
            // `T_old` (epoch 1) the sweep is eligible to drop once the fleet
            // cursor moves strictly past it.
            seed_or_map(&factory, map, key, &["R2"], &["T_old"]).await;
            frontier.stamp_tombstone(map, key, "T_old"); // epoch 1 (this key)
            frontier.stamp_tombstone(map, "other", "T2"); // epoch 2, unrelated key
            track_client(&frontier, &client, conn, 2).await; // LWM 2 > epoch 1 → eligible
            frontier.set_durable_epoch_watermark(1000); // protection active
            assert!(
                frontier.is_epoch_prune_eligible(1),
                "epoch 1 is prune-eligible"
            );

            // Race, started simultaneously, both mutating THIS key's OR-Map state:
            //   (A) an admitted push that unions in a NEW tombstone `R1` (and a
            //       suppressed R1 record), and
            //   (B) the prune sweep that drops `T_old`.
            // Both take the per-key single writer (342d). Serialization must yield
            // exactly {R2 live, tombstones = [R1]} — a torn read would either lose
            // the prune (T_old survives) or lose the push's union (R1 missing).
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let push_svc = Arc::clone(&svc);
            let bp = Arc::clone(&barrier);
            let push = tokio::spawn(async move {
                bp.wait();
                push_svc
                    .oneshot(push_entry(make_ctx_conn(conn), map, key, &["R1"], &["R1"]))
                    .await
                    .expect("ack");
            });
            let (pf, pfac, pkw, bq) = (
                Arc::clone(&frontier),
                Arc::clone(&factory),
                Arc::clone(&key_writer),
                Arc::clone(&barrier),
            );
            let prune = tokio::spawn(async move {
                bq.wait();
                prune_epoch_tombstones(&pf, &pfac, &pkw).await;
            });
            let _ = tokio::join!(push, prune);

            let (mut live, mut tombs) = stored_or_map(&factory, map, key).await;
            live.sort();
            tombs.sort();
            assert_eq!(
                live,
                vec!["R2".to_string()],
                "round {round}: the pre-existing live record survives; R1 stays suppressed (no resurrection)"
            );
            assert_eq!(
                tombs,
                vec!["R1".to_string()],
                "round {round}: per-key writer serialized commit vs prune — T_old pruned AND the pushed union survived (no lost update)"
            );
        }
    }
}
