//! Durable OR-Map recovery under fault injection.
//!
//! Two faults, one oracle. A two-node cluster churns uniquely-tagged OR adds
//! and an observed-remove on both sides of a **network partition**, then heals
//! and Merkle-syncs; the converged value is the semantic truth the CRDT algebra
//! demands. A single durable node then applies the SAME churn through a
//! write-behind store over a real file WAL and suffers a **node failure** — the
//! store is dropped before its buffer ever reaches the backend, so the on-disk
//! WAL is the only surviving copy. Recovery replays that WAL into a FRESH
//! backend, and the reconstructed OR-Map must carry exactly the live entries and
//! the tombstone the partitioned cluster converged on.
//!
//! # What this discriminates
//!
//! The assertions are on OR-Map CONTENTS, never on how a mutation was framed on
//! disk. A recovery fold that stopped seeding from the durable store would
//! rebuild the key from nothing and lose every earlier add; one that replayed
//! out of sequence order would let the observed-removed tag resurrect. Both are
//! semantic outcomes, so both turn this test RED without it ever inspecting a
//! frame.
//!
//! The scenario is additionally run under BOTH settings of the store's OR
//! framing switch and the two recovered states are required to agree. That is
//! the recovery-equivalence contract stated as a differential: whatever the
//! store writes for an OR mutation, what comes back after a crash must be the
//! same live set, the same tombstone set (`TG-OR-003`) — semantic-set equal, not
//! byte-for-byte.
//!
//! # Cells shared with the write-behind queue
//!
//! An OR write shares ONE slot cell between the engine, its queued entry and
//! its staging slot, so a flush encodes whatever the cell holds when the flush
//! runs (`TG-WB-003`). Two more node-failure cases pin that down against the
//! same oracle, under both framing settings, with a backend that SURVIVES the
//! crash (the flushed state is part of what recovery starts from):
//!
//! - a flush of the key's older entry runs while a later op's frame is on disk
//!   but unresolved, so the backend leads the resolved frames; recovery re-folds
//!   that frame over a base that already holds it;
//! - the pending key is evicted and the next OR write re-adopts its staged cell
//!   before the flush, so the flush persists the adopted cell, not a
//!   re-materialized copy that lacks the pending writes.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use tokio::sync::Notify;
    use tower::Service;

    use topgun_core::messages::sync::ClientOpMessage;
    use topgun_core::{ClientOp, ORMapRecord, SystemClock, Timestamp, HLC};

    use crate::network::connection::ConnectionRegistry;
    use crate::service::domain::query::QueryRegistry;
    use crate::service::domain::{CrdtService, SchemaService};
    use crate::service::operation::{service_names, CallerOrigin, Operation, OperationContext};
    use crate::service::security::{SecurityConfig, WriteAdmission};
    use crate::sim::cluster::SimCluster;
    use crate::storage::datastores::{
        RedbDataStore, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
    };
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;
    use crate::storage::map_data_store::MapDataStore;
    use crate::storage::record::RecordValue;
    use crate::storage::wal::{Wal, WalEntry, WalFsyncPolicy, WalRecovery, WalWriter};

    const MAP: &str = "ormap";
    const KEY: &str = "doc";
    const DURABLE_NODE: &str = "durable-node";
    /// Adds per side of the partition.
    const ADDS_PER_SIDE: usize = 5;
    /// The tag added on side A and then observed-removed while still isolated.
    const REMOVED_TAG: &str = "a2";

    // -----------------------------------------------------------------------
    // Semantic projections of an OR-Map value
    // -----------------------------------------------------------------------

    /// Live `tag -> value` entries of an OR-Map value.
    ///
    /// Values are rendered with `Debug` because `Value` carries no `PartialEq`
    /// obligation here and the comparison only needs a stable projection of the
    /// same representation on both sides.
    fn live_entries(value: &RecordValue) -> BTreeMap<String, String> {
        match value {
            RecordValue::OrMap { records, .. } => records
                .iter()
                .map(|e| (e.tag.clone(), format!("{:?}", e.value)))
                .collect(),
            _ => BTreeMap::new(),
        }
    }

    /// Observed-remove tombstone tags of an OR-Map value.
    fn tombstone_tags(value: &RecordValue) -> BTreeSet<String> {
        match value {
            RecordValue::OrMap { tombstones, .. } => tombstones.iter().cloned().collect(),
            RecordValue::OrTombstones { tags } => tags.iter().cloned().collect(),
            RecordValue::Lww { .. } => BTreeSet::new(),
        }
    }

    // -----------------------------------------------------------------------
    // A single durable node: the real CRDT write path over write-behind + WAL
    // -----------------------------------------------------------------------

    /// Drives real `OR_ADD` / `OR_REMOVE` client ops through `CrdtService` onto a
    /// caller-supplied backing store, so the durable half exercises the same
    /// mutation point the cluster half does rather than a hand-rolled stand-in.
    struct DurableNode {
        crdt: Arc<CrdtService>,
        factory: Arc<RecordStoreFactory>,
    }

    impl DurableNode {
        fn build(store: Arc<dyn MapDataStore>) -> Self {
            let hlc = Arc::new(Mutex::new(HLC::new(
                DURABLE_NODE.to_string(),
                Box::new(SystemClock),
            )));
            let write_validator = Arc::new(WriteAdmission::new(
                Arc::new(SecurityConfig::default()),
                hlc,
            ));
            let factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                store,
                Vec::new(),
            ));
            let crdt = Arc::new(CrdtService::new(
                Arc::clone(&factory),
                Arc::new(ConnectionRegistry::new()),
                write_validator,
                Arc::new(QueryRegistry::new()),
                Arc::new(SchemaService::new()),
            ));
            Self { crdt, factory }
        }

        /// Evicts every clean resident record of the churned key's store; a key
        /// whose write is still queued is clean once the store marked it stored.
        fn evict_all_resident(&self) -> u32 {
            self.factory
                .get_or_create(MAP, topgun_core::hash_to_partition(KEY))
                .evict_lru(u32::MAX, false)
        }

        /// Applies one client op, mirroring `SimCluster`'s system-origin call
        /// shape (no connection id, so client auth/ACL is skipped).
        async fn apply(&self, payload: ClientOp) {
            let ts = Timestamp {
                millis: 0,
                counter: 0,
                node_id: DURABLE_NODE.to_string(),
            };
            let mut ctx = OperationContext::new(0, service_names::CRDT, ts, 5000);
            ctx.partition_id = Some(topgun_core::hash_to_partition(KEY));
            ctx.caller_origin = CallerOrigin::System;

            let mut svc = Arc::clone(&self.crdt);
            Service::call(
                &mut svc,
                Operation::ClientOp {
                    ctx,
                    payload: ClientOpMessage { payload },
                },
            )
            .await
            .expect("durable node must ack the OR op");
        }
    }

    /// `OR_ADD` client op: `or_record` + `or_tag` both set.
    fn or_add_op(tag: &str, value: &str) -> ClientOp {
        ClientOp {
            id: Some(format!("{MAP}/{KEY}/{tag}")),
            map_name: MAP.to_string(),
            key: KEY.to_string(),
            op_type: None,
            record: None,
            or_record: Some(Some(ORMapRecord {
                value: rmpv::Value::String(value.into()),
                timestamp: Timestamp {
                    millis: 0,
                    counter: 0,
                    node_id: DURABLE_NODE.to_string(),
                },
                tag: tag.to_string(),
                ttl_ms: None,
            })),
            or_tag: Some(Some(tag.to_string())),
            write_concern: None,
            timeout: None,
        }
    }

    /// `OR_REMOVE` client op: `or_tag` alone, no record.
    fn or_remove_op(tag: &str) -> ClientOp {
        ClientOp {
            id: Some(format!("{MAP}/{KEY}/{tag}#remove")),
            map_name: MAP.to_string(),
            key: KEY.to_string(),
            op_type: None,
            record: None,
            or_record: None,
            or_tag: Some(Some(tag.to_string())),
            write_concern: None,
            timeout: None,
        }
    }

    /// Never-flush write-behind: the buffer must still hold every write when the
    /// crash lands, so the WAL is the only surviving copy and recovery is the
    /// only thing that can produce the post-crash state.
    fn never_flush_config(or_delta_wal: bool) -> WriteBehindConfig {
        WriteBehindConfig {
            flush_interval_ms: 600_000,
            or_delta_wal,
            ..WriteBehindConfig::default()
        }
    }

    // -----------------------------------------------------------------------
    // Fault scenarios
    // -----------------------------------------------------------------------

    /// Fault A — network partition. Two isolated nodes churn concurrently, then
    /// heal and Merkle-sync both directions. Returns the converged value, which
    /// serves as the semantic oracle for the durable half.
    async fn converged_state_across_partition() -> RecordValue {
        let mut cluster = SimCluster::new(2, 4_919);
        cluster.start().expect("cluster should start");

        cluster.inject_partition(&[0], &[1]);

        for i in 0..ADDS_PER_SIDE {
            cluster
                .or_write(
                    0,
                    MAP,
                    KEY,
                    format!("a{i}"),
                    rmpv::Value::String(format!("v{i}").into()),
                )
                .await
                .expect("or_write on partitioned node 0 should succeed");
            cluster
                .or_write(
                    1,
                    MAP,
                    KEY,
                    format!("b{i}"),
                    rmpv::Value::String(format!("v{i}").into()),
                )
                .await
                .expect("or_write on partitioned node 1 should succeed");
        }

        cluster
            .or_remove(0, MAP, KEY, REMOVED_TAG)
            .await
            .expect("or_remove on partitioned node 0 should succeed");

        cluster.heal_partition();
        cluster
            .merkle_sync_pair(0, 1, MAP)
            .await
            .expect("merkle sync 0→1 after heal should succeed");
        cluster
            .merkle_sync_pair(1, 0, MAP)
            .await
            .expect("merkle sync 1→0 after heal should succeed");

        cluster
            .assert_converged(MAP, KEY)
            .await
            .expect("assert_converged should not error")
            .expect("both nodes should hold a value after convergence")
    }

    /// Fault B — node failure. The same churn is applied through the real CRDT
    /// write path onto a write-behind store over a file WAL; the store is then
    /// dropped with its buffer still full (a `kill -9` inside the write-behind
    /// window), and the surviving WAL is replayed into a FRESH backend. Returns
    /// the reconstructed OR-Map.
    /// Total bytes the WAL segments under `dir` occupy on disk.
    ///
    /// Deliberately a BYTE total and never a frame-variant name: this module is
    /// held to semantic assertions, and the belts forbid it naming the variant.
    /// Bytes are also the property that actually matters — the whole point of
    /// per-op framing is what lands on disk, so measuring it directly is
    /// stronger than recognising a discriminant.
    fn wal_bytes_on_disk(dir: &std::path::Path) -> u64 {
        std::fs::read_dir(dir)
            .expect("the wal directory must be readable")
            .flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.metadata().map(|m| m.len()).unwrap_or(0))
            .sum()
    }

    /// Returns the reconstructed OR-Map and the bytes its churn put on disk.
    async fn churn_then_crash_and_recover(or_delta_wal: bool) -> (RecordValue, u64) {
        let wal_dir = tempfile::tempdir().expect("wal tempdir");
        let pre_crash_dir = tempfile::tempdir().expect("pre-crash backend tempdir");
        let recovered_dir = tempfile::tempdir().expect("recovered backend tempdir");

        // PerOp fsync so every ack in the churn below is already on disk when
        // the crash lands: the oracle is "no acked mutation is lost", and a
        // group-commit window would blur which acks that covers.
        let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
            .expect("WalWriter::new");

        let pre_crash_backend: Arc<dyn MapDataStore> = Arc::new(
            RedbDataStore::new(pre_crash_dir.path().join("pre-crash.redb"))
                .expect("pre-crash redb should open"),
        );
        let store = WriteBehindDataStore::new_with_wal(
            pre_crash_backend,
            never_flush_config(or_delta_wal),
            Some(WalBootstrap {
                wal: Arc::clone(&wal) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );

        {
            let node = DurableNode::build(Arc::clone(&store) as Arc<dyn MapDataStore>);
            for i in 0..ADDS_PER_SIDE {
                node.apply(or_add_op(&format!("a{i}"), &format!("v{i}")))
                    .await;
                node.apply(or_add_op(&format!("b{i}"), &format!("v{i}")))
                    .await;
            }
            node.apply(or_remove_op(REMOVED_TAG)).await;
        }

        // The crash: the resident record set and the un-flushed buffer go away
        // together. The pre-crash backend is deliberately NOT reused below, so
        // nothing but the WAL can carry the churn across.
        drop(store);

        // Measured after the crash and before replay, so it is exactly what the
        // churn wrote — replay adds nothing to this directory.
        let wal_bytes = wal_bytes_on_disk(wal_dir.path());

        let recovered_backend: Arc<dyn MapDataStore> = Arc::new(
            RedbDataStore::new(recovered_dir.path().join("recovered.redb"))
                .expect("recovered redb should open"),
        );
        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&recovered_backend))
            .await
            .expect("recovery must succeed on an intact WAL");

        let recovered = recovered_backend
            .load(MAP, KEY)
            .await
            .expect("recovered backend load should not error")
            .expect("every acked OR mutation was durable, so the key must exist after recovery");

        (recovered, wal_bytes)
    }

    // -----------------------------------------------------------------------
    // The proof
    // -----------------------------------------------------------------------

    /// The expected live tags: every tag added on either side of the partition,
    /// minus the one that was observed-removed.
    fn expected_live_tags() -> BTreeSet<String> {
        (0..ADDS_PER_SIDE)
            .flat_map(|i| [format!("a{i}"), format!("b{i}")])
            .filter(|tag| tag != REMOVED_TAG)
            .collect()
    }

    /// OR churn under a network partition AND a crash: the state a node
    /// rebuilds from its WAL must be the state the partitioned cluster
    /// converged on — add-wins for every concurrent tag, remove-wins for the
    /// observed-removed one.
    #[tokio::test(flavor = "multi_thread")]
    async fn or_churn_recovers_to_the_partition_converged_state() {
        let converged = converged_state_across_partition().await;
        let converged_live = live_entries(&converged);
        let converged_tombs = tombstone_tags(&converged);

        // The oracle itself must be the state the algebra demands, or the
        // comparison below would be self-fulfilling.
        assert_eq!(
            converged_live.keys().cloned().collect::<BTreeSet<_>>(),
            expected_live_tags(),
            "partition oracle: every concurrently added tag but the removed one must be live \
             (live={converged_live:?})"
        );
        assert!(
            converged_tombs.contains(REMOVED_TAG),
            "partition oracle: the removed tag must be tombstoned (tombstones={converged_tombs:?})"
        );

        let (recovered, _) = churn_then_crash_and_recover(true).await;
        let recovered_live = live_entries(&recovered);
        let recovered_tombs = tombstone_tags(&recovered);

        // Add-wins across the crash: a fold that lost its base would rebuild the
        // key from the last mutation alone and drop every earlier add here.
        assert_eq!(
            recovered_live, converged_live,
            "recovered live entries must equal the converged live entries \
             (recovered={recovered_live:?}, converged={converged_live:?})"
        );
        // Remove-wins across the crash: a fold that replayed out of order would
        // let the removed tag resurrect here.
        assert!(
            !recovered_live.contains_key(REMOVED_TAG),
            "the observed-removed tag must not resurrect through recovery \
             (recovered={recovered_live:?})"
        );
        assert!(
            recovered_tombs.contains(REMOVED_TAG),
            "the observed-removed tag must survive recovery as a tombstone \
             (tombstones={recovered_tombs:?})"
        );
    }

    /// Recovery-equivalence as a differential: the store's OR framing switch may
    /// change what goes onto disk, but never what comes back. Both settings run
    /// the identical churn through the identical crash, and the two recovered
    /// states must be semantic-set equal (`TG-OR-003`) — not byte-for-byte, so
    /// the comparison is over the live and tombstone sets, not the encoding.
    #[tokio::test(flavor = "multi_thread")]
    async fn or_recovery_is_invariant_under_the_framing_switch() {
        let (armed, armed_bytes) = churn_then_crash_and_recover(true).await;
        let (rolled_back, snapshot_bytes) = churn_then_crash_and_recover(false).await;

        assert_eq!(
            live_entries(&armed),
            live_entries(&rolled_back),
            "recovered live entries must not depend on how the store framed the OR writes"
        );
        assert_eq!(
            tombstone_tags(&armed),
            tombstone_tags(&rolled_back),
            "recovered tombstones must not depend on how the store framed the OR writes"
        );
        // Guard against both sides being vacuously empty: a recovery that
        // produced nothing at all would satisfy the equalities above.
        assert_eq!(
            live_entries(&armed)
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>(),
            expected_live_tags(),
            "the armed run must actually have recovered the churn, not an empty map"
        );

        // The equalities above are the OBLIGATION; this is what stops them being
        // vacuous. If the write path ever stopped delivering a witness, both runs
        // would frame full snapshots, every semantic set would still match, and
        // the differential would pass while asserting nothing about framing at
        // all. The two runs must therefore be observably DIFFERENT on disk.
        assert!(
            armed_bytes < snapshot_bytes,
            "the armed run put {armed_bytes} B on disk and the rolled-back run \
             {snapshot_bytes} B: identical totals mean both runs framed the same way, so the \
             equalities above compared a store against itself"
        );
    }

    // -----------------------------------------------------------------------
    // Cells shared with the write-behind queue, over a surviving backend
    // -----------------------------------------------------------------------

    /// Upper bound on any wait for the store to reach a state the test drives.
    const WAIT_BOUND: Duration = Duration::from_secs(5);

    /// A WAL that holds ONE armed append after its frame is on disk and before
    /// the append returns. The held op has mutated its cell and framed itself,
    /// but it is neither acked nor enqueued, so no flush can have resolved its
    /// frame yet.
    struct HoldingWal {
        inner: Arc<WalWriter>,
        /// The partition of the latest append: the churn writes one key, and the
        /// store picks that key's WAL partition itself.
        partition: AtomicU32,
        armed: AtomicBool,
        held: Notify,
        release: Notify,
    }

    impl HoldingWal {
        fn new(inner: Arc<WalWriter>) -> Self {
            Self {
                inner,
                partition: AtomicU32::new(u32::MAX),
                armed: AtomicBool::new(false),
                held: Notify::new(),
                release: Notify::new(),
            }
        }
    }

    #[async_trait]
    impl Wal for HoldingWal {
        async fn append(&self, partition: u32, entry: &WalEntry) -> anyhow::Result<()> {
            self.inner.append(partition, entry).await?;
            self.partition.store(partition, Ordering::SeqCst);
            if self.armed.swap(false, Ordering::SeqCst) {
                self.held.notify_one();
                self.release.notified().await;
            }
            Ok(())
        }

        async fn mark_applied(&self, partition: u32, sequence: u64) -> anyhow::Result<()> {
            self.inner.mark_applied(partition, sequence).await
        }

        async fn unapplied(&self, partition: u32) -> anyhow::Result<Vec<WalEntry>> {
            self.inner.unapplied(partition).await
        }
    }

    /// Where the flush lands relative to the churn's last add.
    #[derive(Clone, Copy, Debug)]
    enum CellCase {
        /// The key's older entry is flushed while the last add's frame is on
        /// disk but unresolved.
        FlushLeadsAnUnresolvedFrame,
        /// The pending key is evicted, the last add re-adopts its staged cell,
        /// and only then is the key flushed.
        EvictThenReadopt,
    }

    /// Requests a flush and waits until nothing is queued AND the WAL's applied
    /// watermark has moved past every flushed frame, leaving exactly
    /// `unresolved` frames for a replay — so the crash that follows sees the
    /// watermark the flush earned, not one still being written.
    async fn flush_everything_queued(
        store: &WriteBehindDataStore,
        wal: &HoldingWal,
        unresolved: usize,
    ) {
        store.soft_flush().await.expect("soft_flush");
        let partition = wal.partition.load(Ordering::SeqCst);
        assert_ne!(
            partition,
            u32::MAX,
            "precondition: the churn has appended frames"
        );
        let deadline = tokio::time::Instant::now() + WAIT_BOUND;
        loop {
            let unapplied = wal
                .inner
                .unapplied(partition)
                .await
                .expect("unapplied")
                .len();
            if store.pending_operation_count() == 0 && unapplied == unresolved {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the flush: {} queued, {unapplied} unapplied frames, \
                 {unresolved} expected",
                store.pending_operation_count()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// The churn's last add, `b{ADDS_PER_SIDE - 1}`.
    fn last_add() -> ClientOp {
        let last = ADDS_PER_SIDE - 1;
        or_add_op(&format!("b{last}"), &format!("v{last}"))
    }

    /// Holds the last add inside its append (frame on disk, unresolved), flushes
    /// the key's older entry, then releases the add. Returns a violation if the
    /// flush did not persist the held add, i.e. did not encode the cell's state
    /// at flush time.
    async fn flush_while_the_last_add_is_held(
        node: &Arc<DurableNode>,
        store: &WriteBehindDataStore,
        holding: &HoldingWal,
        backend: &Arc<dyn MapDataStore>,
    ) -> Option<String> {
        let last_tag = format!("b{}", ADDS_PER_SIDE - 1);
        holding.armed.store(true, Ordering::SeqCst);
        let held = {
            let node = Arc::clone(node);
            tokio::spawn(async move { node.apply(last_add()).await })
        };
        tokio::time::timeout(WAIT_BOUND, holding.held.notified())
            .await
            .expect("the last add must reach its held append");

        // The held add's frame is on disk and stays unresolved.
        flush_everything_queued(store, holding, 1).await;
        let lead = backend.load(MAP, KEY).await.expect("backend load");
        let lead_live = lead.as_ref().map(live_entries).unwrap_or_default();

        holding.release.notify_one();
        held.await
            .expect("the held add must be acked once released");

        (!lead_live.contains_key(&last_tag)).then(|| {
            format!(
                "the flush must encode the cell's state at flush time, which already carries \
                 {last_tag} while {last_tag}'s frame is unresolved (persisted={lead_live:?})"
            )
        })
    }

    /// Evicts the pending key, lets the last add materialize it, then flushes.
    /// Returns a violation if that add did not re-adopt the staged cell its
    /// queued entry pinned.
    async fn evict_then_readopt(
        node: &DurableNode,
        store: &WriteBehindDataStore,
        holding: &HoldingWal,
    ) -> Option<String> {
        let pinned = store
            .test_pending_cell(MAP, KEY)
            .expect("precondition: an OR write queues its cell");
        assert!(
            node.evict_all_resident() > 0,
            "precondition: the pending key is evicted"
        );
        node.apply(last_add()).await;
        let staged = store
            .test_staged_cell(MAP, KEY)
            .expect("the re-adopting write stages its cell");
        let queued = store
            .test_pending_cell(MAP, KEY)
            .expect("the re-adopting write queues its cell");
        let readopted = Arc::ptr_eq(&staged, &pinned) && Arc::ptr_eq(&queued, &pinned);
        drop((staged, queued, pinned));
        flush_everything_queued(store, holding, 0).await;
        (!readopted).then(|| {
            "the write on the evicted pending key must re-adopt the staged cell that its \
             queued entry pins"
                .to_string()
        })
    }

    /// The churn of [`churn_then_crash_and_recover`] with the last add `b4`
    /// placed per `case`, over a backend that survives the crash. Recovery
    /// replays the WAL into that SAME backend, so what the flush persisted is
    /// part of the base the replay folds over.
    ///
    /// Returns the recovered value and every violated precondition of `case`.
    /// The preconditions are checked before the crash but reported after it, so
    /// a broken cell is judged first by what recovery then loses.
    async fn cell_churn_then_crash_and_recover(
        or_delta_wal: bool,
        case: CellCase,
    ) -> (RecordValue, Vec<String>) {
        let wal_dir = tempfile::tempdir().expect("wal tempdir");
        let backend_dir = tempfile::tempdir().expect("backend tempdir");

        let wal = WalWriter::new(wal_dir.path().to_path_buf(), WalFsyncPolicy::PerOp)
            .expect("WalWriter::new");
        let holding = Arc::new(HoldingWal::new(Arc::clone(&wal)));
        let backend: Arc<dyn MapDataStore> = Arc::new(
            RedbDataStore::new(backend_dir.path().join("backend.redb"))
                .expect("backend redb should open"),
        );
        // Entries are due at once, but only a requested flush drains them.
        let store = WriteBehindDataStore::new_with_wal(
            Arc::clone(&backend),
            WriteBehindConfig {
                write_delay_ms: 0,
                ..never_flush_config(or_delta_wal)
            },
            Some(WalBootstrap {
                wal: Arc::clone(&holding) as Arc<dyn Wal>,
                sequence_start: 1,
            }),
        );

        let mut violations = Vec::new();
        {
            let node = Arc::new(DurableNode::build(
                Arc::clone(&store) as Arc<dyn MapDataStore>
            ));
            for i in 0..ADDS_PER_SIDE {
                node.apply(or_add_op(&format!("a{i}"), &format!("v{i}")))
                    .await;
                if i + 1 < ADDS_PER_SIDE {
                    node.apply(or_add_op(&format!("b{i}"), &format!("v{i}")))
                        .await;
                }
            }
            let violation = match case {
                CellCase::FlushLeadsAnUnresolvedFrame => {
                    flush_while_the_last_add_is_held(&node, &store, &holding, &backend).await
                }
                CellCase::EvictThenReadopt => evict_then_readopt(&node, &store, &holding).await,
            };
            violations.extend(violation);

            // Framed and acked after the flush: its frame is unresolved at the
            // crash, so only the replay can carry it across.
            node.apply(or_remove_op(REMOVED_TAG)).await;
        }

        drop(store);

        WalRecovery::new(Arc::clone(&wal), Vec::new())
            .run(Arc::clone(&backend))
            .await
            .expect("recovery must succeed on an intact WAL");
        let recovered = backend
            .load(MAP, KEY)
            .await
            .expect("backend load after recovery")
            .expect("every acked OR mutation was durable, so the key must exist after recovery");
        (recovered, violations)
    }

    /// Runs `case` under both framing settings and holds each recovered state
    /// to the partition-converged live set and tombstones.
    async fn cell_case_recovers_to_the_partition_converged_state(case: CellCase) {
        let converged = converged_state_across_partition().await;
        let converged_live = live_entries(&converged);
        let converged_tombs = tombstone_tags(&converged);
        assert_eq!(
            converged_live.keys().cloned().collect::<BTreeSet<_>>(),
            expected_live_tags(),
            "partition oracle: every concurrently added tag but the removed one must be live"
        );

        for or_delta_wal in [true, false] {
            let (recovered, violations) =
                cell_churn_then_crash_and_recover(or_delta_wal, case).await;
            assert_eq!(
                live_entries(&recovered),
                converged_live,
                "{case:?} (or_delta_wal={or_delta_wal}): recovered live entries must equal the \
                 converged live entries"
            );
            assert_eq!(
                tombstone_tags(&recovered),
                converged_tombs,
                "{case:?} (or_delta_wal={or_delta_wal}): recovered tombstones must equal the \
                 converged tombstones"
            );
            assert!(
                violations.is_empty(),
                "{case:?} (or_delta_wal={or_delta_wal}): {violations:?}"
            );
        }
    }

    /// A flush may LEAD the resolved frames: it encodes a cell that already
    /// holds an op whose frame is on disk but unresolved. After a node failure
    /// the replay re-folds that frame over the leading backend, and the result
    /// is still exactly the partition-converged state (`TG-WB-003`,
    /// `TG-WAL-011`, `TG-OR-003`).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_flush_that_leads_an_unresolved_frame_recovers_to_the_partition_converged_state() {
        cell_case_recovers_to_the_partition_converged_state(CellCase::FlushLeadsAnUnresolvedFrame)
            .await;
    }

    /// Eviction drops only the engine's reference to a pending key's cell; the
    /// next OR write re-adopts that same cell, so the flush persists every
    /// pending add rather than a copy re-materialized from a backend that has
    /// none of them. After a node failure the recovered state is the
    /// partition-converged one (`TG-WB-003`, `TG-OR-007`).
    #[tokio::test(flavor = "multi_thread")]
    async fn an_evicted_pending_key_re_adopted_before_its_flush_recovers_to_the_partition_converged_state(
    ) {
        cell_case_recovers_to_the_partition_converged_state(CellCase::EvictThenReadopt).await;
    }
}
