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
    use crate::tombstone_frontier_impl::{TombstoneFrontier, TombstoneRef};

    // ------------------------------------------------------------------
    // Harness (mirrors the gated SyncService setup used by the sync-path
    // tests; the `sim` module cannot import their `#[cfg(test)]` private
    // helpers, so it reconstructs the minimal scaffold here and also hands
    // back the shared `KeyWriterRegistry` for the interleaved-prune test).
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

            // R1 was observed and then REMOVED — while its tombstone is present it
            // suppresses any re-add via remove-wins carry-tombstone merge (the
            // `lagging_known_replica_*` sibling proves that leg directly).
            seed_or_map(&factory, map, key, &[], &["R1"]).await;
            frontier.stamp_tombstone(map, key, "R1"); // current_epoch = 1
            assert!(
                stored_or_map(&factory, map, key)
                    .await
                    .1
                    .contains(&"R1".to_string()),
                "precondition: R1's removal tombstone is present"
            );

            // The forgotten client fell past retention, so nothing pins epoch 1 and
            // R1's tombstone is GC'd. Model the completed prune as the resulting
            // empty key. (In production a pruned key only exists once the durable-
            // epoch watermark is set, which ALSO arms the gate — so this pruned-yet-
            // dark state is a deliberate counterfactual isolating the degradation the
            // watermark coupling prevents.)
            seed_or_map(&factory, map, key, &[], &[]).await;

            // The forgotten client returns and re-pushes the removed R1.
            Arc::clone(&svc)
                .oneshot(push_entry(make_ctx_conn(conn), map, key, &["R1"], &[]))
                .await
                .expect("ack");
            let (live, _t) = stored_or_map(&factory, map, key).await;
            assert!(
                live.contains(&"R1".to_string()),
                "ABSENT the gate, a forgotten client's stale push resurrects the removed-then-pruned value (documented degradation)"
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
            // Both take the per-key single writer. Serialization must yield
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

    // ==================================================================
    // Tier-1 deterministic discrimination — the fault-dimension arm
    // (R7.1/R7.1a, zero-cascade exemption shape 4). The per-epoch residency
    // ledger — not just the no-resurrection invariant `AC6` already proves —
    // must stay CORRECT under the SAME interleaved push/prune race: exactly
    // one attributed `DrainedByPrune` exit (never lost to a race-induced
    // `Unclassified`, never double-counted), and O-0's conservation identity
    // still holds. This is the fault surface the file actually reaches (see
    // the module doc's `SimCluster` bypass note) — the interleaving-fault
    // dimension, not a `SimNetwork` partition/delay/reorder, which this
    // file's own scaffold cannot drive.
    // ==================================================================

    /// PHASE 1 (this segment): the fault-dimension arm of the Tier-1
    /// harness. Under the same TOCTOU race
    /// `interleaved_prune_during_push_never_resurrects` proves data-safe,
    /// the residency ledger's own classification-relevant counters must be
    /// UNPERTURBED: the raced epoch is attributed `DrainedByPrune` exactly
    /// once, and O-0 holds over the resulting snapshot.
    ///
    /// PHASE 2 (reserved for a later segment, engaged only if R7.3(c)
    /// fires): the reproducing test itself, if the mechanism's siting rule
    /// lands it at the interleaving-fault boundary. Not present here — this
    /// fn carries ONLY the fault-dimension arm in this segment.
    ///
    /// This file's current contract permits new fns and module-scope helpers
    /// beside this one — it is counted against the increment that governs
    /// its scope, so a second phase, if it lands, may extend this fn's body
    /// or add a sibling fn, whichever the reproducing test's own shape
    /// calls for.
    ///
    /// This fn IS PD-F12's drive `D4` **TOCTOU RACE** — the fixture already in
    /// the file the frozen drive set cites by line. Its predicate is the
    /// AGGREGATE terms `index_conservation_snapshot()` already exposes:
    /// exactly one attributed drain, exactly one exit row, O-0 holds — proven
    /// on every one of the 64 interleavings below, never a lucky one (R4.2).
    /// `D4` is recorded at AGGREGATE granularity ONLY: it runs on a
    /// `multi_thread` runtime, and both in-process observation transports
    /// (`tracing` capture, metrics recorder) are thread-local (C13), so a
    /// prune spawned onto a worker thread is invisible to either — the
    /// per-kind row-count gate R4.6 applies to (`D1`, `D2`, `D3`, `D5`) does
    /// NOT apply here, and this fn does not pretend otherwise by attempting a
    /// row-granularity capture. `D4` is OUTSIDE every D-T row's universe
    /// (Step 2's exclusion table): both in-process transports are
    /// thread-local and its prune runs in a spawned task on a `multi_thread`
    /// runtime (C13, X21-d).
    #[tokio::test(flavor = "multi_thread")]
    async fn prune_epoch_residency_discrimination_under_network_fault() {
        for round in 0..64u64 {
            // ---- PHASE 1: fault-dimension arm --------------------------
            let (svc, factory, frontier, registry, key_writer) = gated_sync();
            let (conn, client) = register_device(&registry, "dev-residency-toctou").await;
            let (map, key) = ("omap", "k1");

            // Same fixture shape as the sibling TOCTOU proof: a live record R2
            // and an old removed tag T_old (epoch 1) the sweep is eligible to
            // drop once the fleet cursor moves strictly past it.
            seed_or_map(&factory, map, key, &["R2"], &["T_old"]).await;
            frontier.stamp_tombstone(map, key, "T_old"); // epoch 1 (this key)
            frontier.stamp_tombstone(map, "other", "T2"); // epoch 2, rolls past epoch 1
            track_client(&frontier, &client, conn, 2).await; // LWM 2 > epoch 1 -> eligible
            frontier.set_durable_epoch_watermark(1000); // protection active
            assert!(
                frontier.is_epoch_prune_eligible(1),
                "round {round}: epoch 1 is prune-eligible"
            );
            let before = frontier.index_conservation_snapshot();

            // Race, started simultaneously, both touching THIS key's OR-Map
            // state: (A) an admitted push unioning in a NEW tombstone R1, and
            // (B) the prune sweep dropping T_old. The shared per-key writer
            // serializes the two — this proof checks the RESIDENCY LEDGER'S
            // OWN bookkeeping survives that serialization intact, not only
            // the stored value (which the sibling test already covers).
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

            let after = frontier.index_conservation_snapshot();
            assert_eq!(
                after.drained_refs_total,
                before.drained_refs_total + 1,
                "round {round}: exactly one attributed DrainedByPrune exit — \
                 never lost to a race-induced Unclassified, never double-counted"
            );
            assert_eq!(
                after.epochs_exited_total,
                before.epochs_exited_total + 1,
                "round {round}: exactly one exit row emitted across the race"
            );
            assert_eq!(
                after.stamped_refs_total + after.restored_refs_total
                    - after.drained_refs_total
                    - after.rebuild_cleared_refs_total,
                after.indexed_refs,
                "round {round}: O-0 must hold across the interleaved fault, got {after:?}"
            );

            let (mut live, mut tombs) = stored_or_map(&factory, map, key).await;
            live.sort();
            tombs.sort();
            assert_eq!(
                live,
                vec!["R2".to_string()],
                "round {round}: no resurrection under the race (AC6's own invariant, re-checked here)"
            );
            assert_eq!(
                tombs,
                vec!["R1".to_string()],
                "round {round}: T_old pruned AND the pushed union survived"
            );
        }
        // D4's own predicate (R4.3), recorded as a value: every one of the 64
        // rounds above asserted the aggregate identity holds (a violation on
        // any round would have already panicked that round's iteration, per
        // R4.4's "surfaced as the test's own failure message" shape) — so
        // reaching this line at all IS the recorded outcome.
        println!(
            "D4 TOCTOU RACE: REPRODUCED — aggregate index_conservation_snapshot() terms \
             (exactly one attributed drain, exactly one exit row, O-0) held across all \
             64 interleavings. Row-granularity terms are NOT observable here (C13, \
             X21-d): the prune runs in a tokio::spawn task on a multi_thread runtime, \
             so the thread-local tracing capture would see an empty capture rather than \
             a violation. NOT in a Decision-Table universe (Step 2's exclusion table)."
        );
    }

    // ==================================================================
    // PD-F12 — the frozen drive set `D1 … D5` (segment (a): the shared
    // `tracing` capture-layer helper plus `D1`–`D3`; `D4`, `D5` and the
    // Decision-Table evaluation are a separate, later segment).
    //
    // Each drive records its outcome as a VALUE (`REPRODUCED` /
    // `NOT-REPRODUCED`) plus the evaluated value of its OWN predicate
    // (R4.3) — no Decision-Table row or verdict is drawn here; that
    // evaluation ranges over the whole frozen drive set and is a separate,
    // later step.
    // ==================================================================

    const RESIDENCY_TARGET: &str = "topgun_server::tombstone_frontier::residency";
    const SETTLEMENT_TARGET: &str = "topgun_server::tombstone_frontier::settlement";

    /// Renders one captured event's fields as `name=value` pairs, of the
    /// `network/device_identity.rs:397-429` shape. `u64` and `bool` fields
    /// reach `record_debug` exactly like every other non-`&str` field —
    /// Debug and Display agree for both, so the rendered text is the same
    /// either way.
    struct RowFieldVisitor(String);

    impl tracing::field::Visit for RowFieldVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{}={value} ", field.name());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write as _;
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }

    /// Sink for [`RowFieldVisitor`]: one rendered line PER CAPTURED EVENT, a
    /// `Vec<String>`, never a single concatenated `String` (X21-d(iv)) —
    /// every drive below reads a SPECIFIC row's own fields off the capture,
    /// which a concatenated blob cannot individuate without inventing its
    /// own newline convention.
    #[derive(Clone)]
    struct RowCapture(Arc<std::sync::Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RowCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut v = RowFieldVisitor(format!(" target={} ", event.metadata().target()));
            event.record(&mut v);
            self.0.lock().unwrap().push(v.0);
        }
    }

    /// Bind a thread-local `tracing` capture for the guard's lifetime. Every
    /// drive that reads rows off it runs on a CURRENT-THREAD runtime (R4.2,
    /// X21-d): `tracing::subscriber::set_default` is thread-local, so a
    /// multi-thread runtime would observe an empty capture instead of a
    /// violation — exactly why `D4`'s race cannot be read at row
    /// granularity (C13).
    fn capture_tracing_rows() -> (
        tracing::subscriber::DefaultGuard,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use tracing_subscriber::layer::SubscriberExt as _;
        let sink: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(RowCapture(Arc::clone(&sink)));
        let guard = tracing::subscriber::set_default(subscriber);
        (guard, sink)
    }

    /// Whether `line` is a row on `target` carrying `kind = "kind"`.
    fn is_row(line: &str, target: &str, kind: &str) -> bool {
        line.contains(&format!(" target={target} ")) && line.contains(&format!(" kind={kind} "))
    }

    /// Read one field's rendered text out of a captured row.
    fn row_field<'a>(line: &'a str, name: &str) -> &'a str {
        let needle = format!(" {name}=");
        let start = line
            .find(&needle)
            .unwrap_or_else(|| panic!("field {name} is absent from row {line:?}"))
            + needle.len();
        line[start..].split(' ').next().unwrap_or("")
    }

    /// Read one `u64`-rendered field out of a captured row.
    fn row_u64(line: &str, name: &str) -> u64 {
        row_field(line, name)
            .parse()
            .unwrap_or_else(|e| panic!("field {name} did not render a u64 in {line:?}: {e}"))
    }

    /// Read one `bool`-rendered field out of a captured row.
    fn row_bool(line: &str, name: &str) -> bool {
        row_field(line, name)
            .parse()
            .unwrap_or_else(|e| panic!("field {name} did not render a bool in {line:?}: {e}"))
    }

    /// Rows on `target` whose `kind` field equals `kind`.
    fn rows_of_kind<'a>(rows: &'a [String], target: &str, kind: &str) -> Vec<&'a str> {
        rows.iter()
            .map(String::as_str)
            .filter(|l| is_row(l, target, kind))
            .collect()
    }

    /// Rows on `target` regardless of `kind` — for the settlement target,
    /// which carries no `kind` field at all (it is distinguished by target
    /// alone).
    fn rows_of_target<'a>(rows: &'a [String], target: &str) -> Vec<&'a str> {
        rows.iter()
            .map(String::as_str)
            .filter(|l| l.contains(&format!(" target={target} ")))
            .collect()
    }

    /// R4.6: the capture must be non-empty and carry the EXPECTED count of
    /// each row kind the caller is about to read, counted separately —
    /// `epoch_exit`, `settlement` and `prune_pass` — never silently short in
    /// any one kind. A drive that skipped this and read a short capture
    /// would report "no violation" on a term it never actually read; this
    /// turns that into a RED instead.
    fn assert_capture_shape(
        rows: &[String],
        expected_epoch_exit: usize,
        expected_settlement: usize,
        expected_prune_pass: usize,
    ) {
        assert!(
            !rows.is_empty(),
            "R4.6: the capture must be non-empty, got an empty capture"
        );
        let exit = rows_of_kind(rows, RESIDENCY_TARGET, "epoch_exit").len();
        assert_eq!(
            exit, expected_epoch_exit,
            "R4.6: expected {expected_epoch_exit} epoch_exit row(s); capture was:\n{rows:#?}"
        );
        let settlement = rows_of_target(rows, SETTLEMENT_TARGET).len();
        assert_eq!(
            settlement, expected_settlement,
            "R4.6: expected {expected_settlement} settlement row(s); capture was:\n{rows:#?}"
        );
        let pass = rows_of_kind(rows, RESIDENCY_TARGET, "prune_pass").len();
        assert_eq!(
            pass, expected_prune_pass,
            "R4.6: expected {expected_prune_pass} prune_pass row(s) — exactly once per \
             driven invocation; capture was:\n{rows:#?}"
        );
    }

    /// Split a capture into successive passes: every row strictly after the
    /// previous pass row up to and including its own pass row (Step 1's
    /// frozen "Pass individuation" rule — the pass row is always emitted
    /// last). Mirrors `service/domain/crdt.rs`'s `split_into_passes`, adapted
    /// to this file's per-event `Vec<String>` capture rather than a single
    /// concatenated string.
    fn split_into_passes(rows: &[String]) -> Vec<Vec<&str>> {
        let mut passes = Vec::new();
        let mut current = Vec::new();
        for line in rows {
            current.push(line.as_str());
            if is_row(line, RESIDENCY_TARGET, "prune_pass") {
                passes.push(std::mem::take(&mut current));
            }
        }
        passes
    }

    // ------------------------------------------------------------------
    // `D1` DIVERGENCE, `D2` POST-EXIT RESTORE, `D3` ABSENT-KEY.
    // ------------------------------------------------------------------

    /// `D1` **DIVERGENCE** — the sim-side twin of the lib's `S1` sanity row
    /// (`tombstone_frontier_impl.rs`'s
    /// `s1_pre_freeze_sanity_row_diverges_through_public_entry_points_only`).
    /// Stamps `N = 2` refs into epoch `e`, rolls past it, restores one EXTRA
    /// ref into `e` (index-only — the entry-side slot is untouched), then
    /// drives the drop through the public `prune_epoch_tombstones` entry
    /// point. NOT in a Decision-Table universe — this IS the Step-0
    /// discriminator (C12), not evidence about the prune.
    ///
    /// Its OWN predicate (R4.3): `R_obs == R_ent + 1` and
    /// `B_obs == B_att + len(extra.tag)` on `e`'s `DrainedByPrune` exit row.
    #[tokio::test]
    async fn d1_divergence_removed_observation_diverges_from_entry_attribution() {
        let (_svc, factory, frontier, registry, key_writer) = gated_sync();
        let (conn, client) = register_device(&registry, "dev-d1").await;
        let (map, key) = ("omap", "k1");
        frontier.set_epoch_width(2); // N = 2 stamps land in epoch 1

        frontier.stamp_tombstone(map, key, "TAG1"); // epoch 1: ref 1 of 2
        frontier.stamp_tombstone(map, key, "TAG2"); // epoch 1: ref 2 of 2
        frontier.stamp_tombstone(map, "k2", "TAG3"); // rolls past epoch 1: refs_at_entry = 2

        let extra = TombstoneRef {
            map: map.to_string(),
            key: key.to_string(),
            tag: "TAGX".to_string(),
        };
        // Index-only reinsert into the ALREADY-ENTRY-EMITTED epoch 1:
        // `epoch_tags[1]` grows to 3 refs while `epoch_slots[1]` (and so
        // `refs_at_entry`) is untouched — this IS the divergence mechanism
        // (C12).
        frontier.restore_tombstone_ref(1, extra.clone());

        track_client(&frontier, &client, conn, 2).await; // LWM 2 > epoch 1
        frontier.set_durable_epoch_watermark(1000);
        seed_or_map(&factory, map, key, &[], &["TAG1", "TAG2", "TAGX"]).await;

        let (guard, sink) = capture_tracing_rows();
        prune_epoch_tombstones(&frontier, &factory, &key_writer).await;
        drop(guard);
        let rows = sink.lock().unwrap().clone();

        // R4.6: exactly one drained epoch (1), so exactly one exit row, one
        // settlement row, and — a single explicit call — exactly one pass
        // row.
        assert_capture_shape(&rows, 1, 1, 1);

        let exit_row = rows_of_kind(&rows, RESIDENCY_TARGET, "epoch_exit")
            .into_iter()
            .find(|r| r.contains(" epoch=1 "))
            .expect("epoch 1's DrainedByPrune exit row must be present");
        assert!(
            exit_row.contains("exit_kind=DrainedByPrune"),
            "row: {exit_row}"
        );

        let r_obs = row_u64(exit_row, "removed_refs_observed");
        let r_ent = row_u64(exit_row, "refs_at_entry");
        let b_obs = row_u64(exit_row, "removed_bytes_observed");
        let b_att = row_u64(exit_row, "bytes_freed_attributed");
        let extra_len = extra.tag.len() as u64;

        // D1's OWN predicate (R4.3), recorded as a value rather than
        // asserted blindly: a failure here is a RESULT (NOT-REPRODUCED),
        // never a reason to weaken the assertion.
        assert_eq!(
            r_obs,
            r_ent + 1,
            "D1 predicate limb 1 (R_obs == R_ent + 1) must hold; row: {exit_row}"
        );
        assert_eq!(
            b_obs,
            b_att + extra_len,
            "D1 predicate limb 2 (B_obs == B_att + len(extra.tag)) must hold; row: {exit_row}"
        );
        println!(
            "D1 DIVERGENCE: REPRODUCED — R_obs={r_obs} R_ent={r_ent} B_obs={b_obs} \
             B_att={b_att} len(extra.tag)={extra_len}"
        );
    }

    /// `D2` **POST-EXIT RESTORE** — drains epoch `e` (its exit row fires,
    /// `epoch_slots[e]` retires for good), restores an EXTRA ref into that
    /// already-EXITED epoch, then re-drains. Confirms C10: a retired slot is
    /// never recreated, so the second drain's detection sweep (which walks
    /// `epoch_slots`, not `epoch_tags`) never sees `e` again, even though
    /// `e`'s restored ref is PHYSICALLY drained a second time. NOT in a
    /// Decision-Table universe.
    ///
    /// Its OWN predicate (R4.3): the SECOND drain's own pass removes
    /// `k > 0` refs with `Σ_e R_obs == 0` (no exit row for `e`) on that SAME
    /// pass.
    #[tokio::test]
    async fn d2_post_exit_restore_reappears_without_a_new_exit_row() {
        let (_svc, factory, frontier, registry, key_writer) = gated_sync();
        let (conn, client) = register_device(&registry, "dev-d2").await;
        let (map, key) = ("omap", "k1");
        frontier.set_epoch_width(1);
        frontier.stamp_tombstone(map, key, "TAG1"); // epoch 1
        frontier.stamp_tombstone(map, "k2", "TAG2"); // rolls past epoch 1

        track_client(&frontier, &client, conn, 2).await; // LWM 2 > epoch 1
        frontier.set_durable_epoch_watermark(1000);

        // FIRST drain: epoch 1 exits — its exit row fires and its
        // `epoch_slots` entry retires.
        let (guard1, sink1) = capture_tracing_rows();
        prune_epoch_tombstones(&frontier, &factory, &key_writer).await;
        drop(guard1);
        let first = sink1.lock().unwrap().clone();
        assert_capture_shape(&first, 1, 1, 1);
        let first_exit = rows_of_kind(&first, RESIDENCY_TARGET, "epoch_exit")
            .into_iter()
            .find(|r| r.contains(" epoch=1 "))
            .expect("epoch 1's first exit row must be present");
        assert!(
            first_exit.contains("exit_kind=DrainedByPrune"),
            "row: {first_exit}"
        );

        // Restore an EXTRA ref into the now-EXITED epoch 1 — index-only; the
        // retired slot is never recreated by a restore.
        let extra = TombstoneRef {
            map: map.to_string(),
            key: key.to_string(),
            tag: "TAGY".to_string(),
        };
        frontier.restore_tombstone_ref(1, extra);

        // SECOND drain, a fresh capture window.
        let (guard2, sink2) = capture_tracing_rows();
        prune_epoch_tombstones(&frontier, &factory, &key_writer).await;
        drop(guard2);
        let second = sink2.lock().unwrap().clone();

        // R4.6: no exit row for the retired epoch — settlement still fires
        // (it is keyed off the physically-drained refs, not off the retired
        // `epoch_slots` tracking) — and one pass row for this single
        // explicit call.
        assert_capture_shape(&second, 0, 1, 1);

        let pass_row = rows_of_kind(&second, RESIDENCY_TARGET, "prune_pass")
            .into_iter()
            .next()
            .expect("the second call's own pass row must be present");
        let k = row_u64(pass_row, "considered");
        let r_obs_sum: u64 = rows_of_kind(&second, RESIDENCY_TARGET, "epoch_exit")
            .iter()
            .map(|r| row_u64(r, "removed_refs_observed"))
            .sum();

        // D2's OWN predicate (R4.3).
        assert!(k > 0, "D2 predicate limb 1 (k > 0) must hold: k={k}");
        assert_eq!(
            r_obs_sum, 0,
            "D2 predicate limb 2 (Σ_e R_obs == 0) must hold on the second pass; \
             capture was:\n{second:#?}"
        );
        println!("D2 POST-EXIT RESTORE: REPRODUCED — k={k} sum_R_obs={r_obs_sum}");
    }

    /// `D3` **ABSENT-KEY** — stamps a tombstone for a key never written to
    /// the store, then drains. The considered ref takes the `Ok(None)` arm
    /// (`crdt.rs:1560-1563`): the drain is real (the index entry is gone,
    /// the exit row's attribution is genuine) but nothing was ever resident
    /// in storage to reclaim. Confirms C9. NOT in a Decision-Table universe.
    ///
    /// Its OWN predicate (R4.3): per-epoch `D_e == 0 ∧ F_e == 0` on the
    /// settlement row while `B_att > 0` on that SAME epoch's exit row.
    #[tokio::test]
    async fn d3_absent_key_settlement_shows_zero_reclaim_despite_positive_attribution() {
        let (_svc, factory, frontier, registry, key_writer) = gated_sync();
        let (conn, client) = register_device(&registry, "dev-d3").await;
        let (map, key) = ("omap", "never-written-k1");
        frontier.set_epoch_width(1);
        frontier.stamp_tombstone(map, key, "TAG1"); // epoch 1 — `key` is never seeded
        frontier.stamp_tombstone(map, "k2", "TAG2"); // rolls past epoch 1

        track_client(&frontier, &client, conn, 2).await; // LWM 2 > epoch 1
        frontier.set_durable_epoch_watermark(1000);
        // Deliberately no `seed_or_map` call: `key` never existed in the store.

        let (guard, sink) = capture_tracing_rows();
        prune_epoch_tombstones(&frontier, &factory, &key_writer).await;
        drop(guard);
        let rows = sink.lock().unwrap().clone();

        assert_capture_shape(&rows, 1, 1, 1);

        let exit_row = rows_of_kind(&rows, RESIDENCY_TARGET, "epoch_exit")
            .into_iter()
            .find(|r| r.contains(" epoch=1 "))
            .expect("epoch 1's DrainedByPrune exit row must be present");
        assert!(
            exit_row.contains("exit_kind=DrainedByPrune"),
            "row: {exit_row}"
        );
        let b_att = row_u64(exit_row, "bytes_freed_attributed");

        let settlement_row = rows_of_target(&rows, SETTLEMENT_TARGET)
            .into_iter()
            .find(|r| r.contains(" epoch=1 "))
            .expect("epoch 1's settlement row must be present");
        let dropped = row_u64(settlement_row, "dropped");
        let bytes_freed = row_u64(settlement_row, "bytes_freed");
        let absent = row_u64(settlement_row, "absent");

        // D3's OWN predicate (R4.3).
        assert_eq!(
            dropped, 0,
            "D3 predicate limb 1 (D_e == 0) must hold; row: {settlement_row}"
        );
        assert_eq!(
            bytes_freed, 0,
            "D3 predicate limb 2 (F_e == 0) must hold; row: {settlement_row}"
        );
        assert!(
            b_att > 0,
            "D3 predicate limb 3 (B_att > 0) must hold; row: {exit_row}"
        );
        assert_eq!(
            absent, 1,
            "the single considered ref must take the Ok(None) absent-key arm: {settlement_row}"
        );
        println!(
            "D3 ABSENT-KEY: REPRODUCED — D_e={dropped} F_e={bytes_freed} B_att={b_att} absent={absent}"
        );
    }

    // ------------------------------------------------------------------
    // `D5` SEQUENTIAL REGIME — the D-T's SOLE UNIVERSE — plus the mechanical
    // per-row Decision-Table evaluation over its recorded rows.
    // ------------------------------------------------------------------

    /// `D5` **SEQUENTIAL REGIME** — the only drive whose rows form a D-T
    /// universe (Step 2). `N = 3000` stamps at `epoch_width = 1` (⇒ 3000
    /// epochs), every key seeded PRESENT with its own tombstone, one tracked
    /// cursor advanced per stamp, one `prune_epoch_tombstones` pass per
    /// stamp.
    ///
    /// **The durable-epoch watermark and the cursor's `delivered` bound are
    /// both `1_000_000`, each set ONCE / re-set idempotently, never the
    /// file's ubiquitous `1000` literal or `track_client`'s hardcoded
    /// `set_delivered(conn, 100)`** (`:204-215`): either literal would clamp
    /// the cursor or the watermark below this drive's `3000` epochs and
    /// silently shrink the D-T's sole universe — the exact hazard the spec's
    /// `D5` paragraphs name. `claimed` is passed as the same `1_000_000`
    /// sentinel on every `confirm_apply_ack`: `advance_on_ack` clamps by
    /// `claimed.min(delivered).min(current_max_epoch)`, and `current_max_\
    /// epoch` already tracks the latest stamped epoch, so the sentinel never
    /// binds — the cursor still advances exactly one epoch per stamp.
    ///
    /// **Closing stamp (deliberate, and NOT one of the N keyed refs).**
    /// `epoch_width = 1` maps `stamp_tombstone`'s `op_seq` directly onto the
    /// epoch number, and ENTRY emission for epoch `e` fires only on the
    /// STAMP that rolls PAST `e` — i.e. on stamp `e + 1`. So after exactly N
    /// keyed stamps, epochs `1 ..= N-1` have entered (and, one pass later
    /// each, drained) but epoch N — the last key's own epoch — is still
    /// OPEN: nothing has stamped past it yet. One additional closing stamp
    /// (a throwaway key, seeding no store entry, never itself drained: its
    /// own epoch stays open at the end) rolls epoch N over so its exit and
    /// settlement rows land inside this capture window too. This is what
    /// makes the capture's `epoch_exit` row count equal the drive's own N
    /// keyed stamps, per R4.6's per-kind gate below — verified here, not
    /// assumed.
    ///
    /// Its OWN predicate (R4.3): **I1, I2, I3 hold on every pass**; **P-KEYS
    /// holds on every considered ref**. A violation is asserted immediately
    /// where it is read (R4.4) — this fn does not "fix the test" on a
    /// failure, it records the violating pass/epoch verbatim in the panic
    /// message. This fn does **not** publish a Decision-Table verdict: it
    /// evaluates and records each row's OWN condition as a value (R4.5),
    /// leaving the naming step to the freeze-gated group that owns it.
    // A single fn deliberately, over the drive's own N=3000 loop plus its
    // closing stamp, the R4.6 capture-shape gate, the I1/I2/I3 + P-KEYS
    // per-pass evaluation and the D-T's per-row evaluation — splitting any
    // of these out would separate an assertion from the capture it reads,
    // which is exactly the shape R4.4/R4.6 need kept together in one place.
    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn d5_sequential_regime_bridge_identities_and_p_keys_hold_over_3000_epochs() {
        const N: u64 = 3000;
        let (_svc, factory, frontier, registry, key_writer) = gated_sync();
        let (conn, client) = register_device(&registry, "dev-d5").await;
        let map = "omap";

        // Set ONCE before the stamp loop, never lowered — exceeds the
        // highest epoch this drive can reach (N + 1) by more than two orders
        // of magnitude.
        frontier.set_durable_epoch_watermark(1_000_000);

        let (guard, sink) = capture_tracing_rows();

        for i in 1..=N {
            let key = format!("d5-k{i}");
            let tag = format!("D5TAG{i}");
            // Every key seeded PRESENT with its own tombstone, before its
            // epoch can possibly roll and drain.
            seed_or_map(&factory, map, &key, &[], &[tag.as_str()]).await;
            frontier.stamp_tombstone(map, &key, &tag);
            // Before EACH confirm_apply_ack, per the drive text.
            frontier.set_delivered(conn, 1_000_000);
            let advanced = frontier.confirm_apply_ack(&client, 1_000_000, conn).await;
            assert!(advanced, "D5: the tracked cursor must advance on stamp {i}");
            // One prune_epoch_tombstones pass per stamp.
            prune_epoch_tombstones(&frontier, &factory, &key_writer).await;
        }

        // The closing stamp: rolls epoch N over so its own exit/settlement
        // rows land inside this capture window (see the doc comment above).
        frontier.stamp_tombstone(map, "d5-closing-key", "D5-CLOSE");
        frontier.set_delivered(conn, 1_000_000);
        let closing_advanced = frontier.confirm_apply_ack(&client, 1_000_000, conn).await;
        assert!(
            closing_advanced,
            "D5: the tracked cursor must advance on the closing stamp"
        );
        prune_epoch_tombstones(&frontier, &factory, &key_writer).await;

        drop(guard);
        let rows = sink.lock().unwrap().clone();

        // R4.6: N draining invocations (all but the very first of the N+1
        // total, which finds nothing yet rolled) produce N epoch_exit rows
        // and N settlement rows (every D5 epoch has exactly one considered
        // ref, so the exit-row/settlement-row asymmetry the Delta names
        // never triggers here); N+1 total invocations produce N+1 pass rows
        // — exactly once per invocation, on every path, per C1's tip-CONFIRMED
        // fact.
        let expected_epoch_exit = usize::try_from(N).expect("N fits in usize");
        let expected_settlement = usize::try_from(N).expect("N fits in usize");
        let expected_prune_pass = usize::try_from(N + 1).expect("N + 1 fits in usize");
        assert_capture_shape(
            &rows,
            expected_epoch_exit,
            expected_settlement,
            expected_prune_pass,
        );

        // Segment (a)'s open question, verified empirically rather than
        // assumed: does `split_into_passes` recover exactly N+1 passes —
        // i.e. does D5's direct-call shape keep the pass-row count 1:1 with
        // this drive's own `prune_epoch_tombstones` invocation count, the
        // way D1–D3 already showed? (G3 separately proved the PUSH path can
        // trigger hidden internal sweeps that break that 1:1 relationship;
        // D5 never routes through `gated_sync()`'s `SyncService` push path,
        // so no such hidden sweep can fire here.)
        let passes = split_into_passes(&rows);
        assert_eq!(
            passes.len(),
            expected_prune_pass,
            "D5: split_into_passes must recover exactly N+1 passes, matching this \
             drive's own N+1 prune_epoch_tombstones invocations 1:1; rows:\n{rows:#?}"
        );

        // Index once, by the `epoch` field's rendered text, for the I3 /
        // P-KEYS / D-T row 2 reads below.
        let exit_rows: Vec<&str> = rows_of_kind(&rows, RESIDENCY_TARGET, "epoch_exit");
        let settlement_rows: Vec<&str> = rows_of_target(&rows, SETTLEMENT_TARGET);
        let mut exit_by_epoch: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        for row in &exit_rows {
            exit_by_epoch.insert(row_field(row, "epoch"), row);
        }
        let mut settlement_by_epoch: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        for row in &settlement_rows {
            settlement_by_epoch.insert(row_field(row, "epoch"), row);
        }

        let mut empty_passes = 0u64;
        let mut draining_passes = 0u64;
        let mut i3_checked = 0u64;
        let mut p_keys_checked = 0u64;
        let mut not_applicable_passes = 0u64;

        for pass_rows in &passes {
            let pass_row = pass_rows
                .last()
                .expect("a pass's own rows always end with its pass row");
            assert!(
                is_row(pass_row, RESIDENCY_TARGET, "prune_pass"),
                "D5: a pass's last row must be its own pass row: {pass_row:?}"
            );
            let k_p = row_u64(pass_row, "considered");
            let e_p = row_bool(pass_row, "empty_drain");

            let exit_rows_in_pass: Vec<&str> = pass_rows
                .iter()
                .copied()
                .filter(|l| is_row(l, RESIDENCY_TARGET, "epoch_exit"))
                .collect();

            // Step 1's scoping: NOT-APPLICABLE on a pass where any
            // `restored_*` exit fired. D5 never restores (no D1/D2-shaped
            // step in this drive), so this is expected to hold vacuously
            // over every pass — recorded as a value below, not assumed.
            let any_restored = exit_rows_in_pass
                .iter()
                .any(|r| row_field(r, "exit_kind").starts_with("Restored"));
            if any_restored {
                not_applicable_passes += 1;
                continue;
            }

            let r_obs_sum: u64 = exit_rows_in_pass
                .iter()
                .map(|r| row_u64(r, "removed_refs_observed"))
                .sum();

            // I1: Σ_e R_obs(e) == K_p, both read off this SAME pass.
            assert_eq!(
                r_obs_sum, k_p,
                "D5 I1 violated on a pass: Σ_e R_obs(e) == K_p must hold; pass rows:\n{pass_rows:?}"
            );
            // I2: E_p == (Σ_e R_obs(e) == 0), E_p off the same pass row.
            assert_eq!(
                e_p,
                r_obs_sum == 0,
                "D5 I2 violated on a pass: empty_drain == (Σ_e R_obs(e) == 0) must hold; \
                 pass rows:\n{pass_rows:?}"
            );

            if e_p {
                empty_passes += 1;
            } else {
                draining_passes += 1;
            }

            // I3 + P-KEYS, per drained epoch in this pass.
            for exit_row in &exit_rows_in_pass {
                let epoch = row_field(exit_row, "epoch");
                let b_obs = row_u64(exit_row, "removed_bytes_observed");
                let b_att = row_u64(exit_row, "bytes_freed_attributed");
                // I3: B_obs(e) <= B_att(e) + B_restored(e). D5's
                // B_restored(e) is 0 on every epoch — this drive never
                // restores.
                assert!(
                    b_obs <= b_att,
                    "D5 I3 violated on epoch {epoch}: B_obs <= B_att + B_restored(0) \
                     must hold; row: {exit_row}"
                );
                i3_checked += 1;

                let settlement_row = settlement_by_epoch.get(epoch).unwrap_or_else(|| {
                    panic!("D5: epoch {epoch}'s settlement row must be present")
                });
                let absent = row_u64(settlement_row, "absent");
                // P-KEYS (precondition, recorded as a VALUE, not assumed):
                // the considered ref's key was present in the store at
                // drain time. Every D5 key is seeded before its epoch can
                // possibly roll, so `absent` is expected 0 on every epoch —
                // a nonzero value here REDs the round rather than silently
                // resolving D-T row 2.
                assert_eq!(
                    absent, 0,
                    "P-KEYS violated on epoch {epoch}: the considered ref's key must be \
                     present in the store at drain time; settlement row: {settlement_row}"
                );
                p_keys_checked += 1;
            }
        }

        assert_eq!(
            not_applicable_passes, 0,
            "D5 never restores: no pass should scope out"
        );
        assert_eq!(
            empty_passes, 1,
            "exactly the first pass (i=1) is an empty drain"
        );
        assert_eq!(
            draining_passes, N,
            "the remaining N passes each drain exactly one epoch"
        );
        assert_eq!(i3_checked, N, "I3 must be checked on all N drained epochs");
        assert_eq!(
            p_keys_checked, N,
            "P-KEYS must be checked on all N considered refs"
        );

        // ------------------------------------------------------------
        // Step 2 — the mechanical per-row Decision-Table evaluation over
        // D5, the D-T's sole universe. Each row's OWN condition is
        // evaluated and recorded as a VALUE below. This is evidence, not a
        // verdict: which row (if any) names this round's reading is a
        // later, freeze-gated determination this group does not make.
        // ------------------------------------------------------------

        // Row 1 (V1 REF-LOSS). Universe: every DrainedByPrune exit row
        // recorded by D5. Condition: exists a row with R_obs < R_ent.
        let row1_condition = exit_rows.iter().any(|r| {
            r.contains("exit_kind=DrainedByPrune")
                && row_u64(r, "removed_refs_observed") < row_u64(r, "refs_at_entry")
        });

        // Row 2 (V2 BOOKKEEPING ATTRIBUTION). Universe: every drained epoch
        // recorded by D5, under precondition P-KEYS (verified above, holding
        // on every considered ref). Condition: exists an epoch with
        // R_obs > 0 AND D_e == 0 AND F_e == 0 AND B_att > 0.
        let row2_condition = settlement_rows.iter().any(|settlement_row| {
            let epoch = row_field(settlement_row, "epoch");
            let dropped = row_u64(settlement_row, "dropped");
            let bytes_freed = row_u64(settlement_row, "bytes_freed");
            let Some(exit_row) = exit_by_epoch.get(epoch) else {
                return false;
            };
            let r_obs = row_u64(exit_row, "removed_refs_observed");
            let b_att = row_u64(exit_row, "bytes_freed_attributed");
            r_obs > 0 && dropped == 0 && bytes_freed == 0 && b_att > 0
        });

        // Row 3 (V3 COUNTER-FAMILY MISNAMING). Universe: every prune pass
        // driven by D5 on which no restored_* exit fired. Condition: I1 or
        // I2 is VIOLATED on at least one such pass. Both were asserted true
        // on every in-scope pass above (R4.4: a violation surfaces as the
        // test's own failure message rather than reaching this line), so
        // this flag is `false` whenever execution reaches here at all.
        let row3_condition = false;

        println!(
            "D5 SEQUENTIAL REGIME: REPRODUCED — I1/I2/I3 held on every in-scope pass \
             ({empty_passes} empty, {draining_passes} draining, {not_applicable_passes} \
             not-applicable), P-KEYS held on {p_keys_checked}/{N} considered refs. \
             D-T row conditions (recorded evidence, NOT a verdict): \
             row1(V1 REF-LOSS)={row1_condition} row2(V2 BOOKKEEPING-ATTRIBUTION)={row2_condition} \
             row3(V3 COUNTER-FAMILY-MISNAMING)={row3_condition}."
        );
    }
}
