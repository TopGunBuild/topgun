//! Allocation proof of the OR write path through the record store.
//!
//! One `OR_ADD` on a resident OR-Map slot of N entries runs through
//! `DefaultRecordStore` over a `WriteBehindDataStore` on redb with the WAL
//! armed for delta framing (`or_delta_wal = true` and a real `WalWriter`), so
//! the mutation carries a witness exactly as on the production path. The only
//! observer attached is the Merkle observer (the leaf hash is the one per-op
//! cost that scales with N by design), so every other per-op byte that scales
//! with N is a whole-record copy.
//!
//! Readings, per case, are medians over repeated ops for the OR cases (a median
//! keeps a one-off container growth — a `DashMap` shard, a queue — out of the
//! reading) and minimums for the LWW cases (see `minimum`):
//!
//! * `c(N)` — bytes of one engine `get` of the slot: one whole-record clone.
//! * resident `OR_ADD` — bytes of one `update_in_place` on the resident slot.
//! * materialize `OR_ADD` — bytes of one `update_in_place` on a key whose
//!   in-place write is still pending in the write-behind store and which was
//!   evicted from the engine before the op.
//! * LWW update of a resident key and LWW insert of a new key — allocation
//!   count and bytes of one `put`.
//!
//! Slopes over N ∈ {1 000, 10 000} cancel every allocation that does not scale
//! with the slot, so `slope / c` counts the whole-record copies of the op.
//!
//! The flush loop is kept out of every window: `write_delay_ms` and
//! `flush_interval_ms` are far above the test's runtime, and the flush loop's
//! first tick is spent during the un-measured warm-up.
//!
//! Only the public API is used, so the same file runs on any revision that
//! keeps it. The predicates asserted depend on `OR_WRITE_ALLOC_SIDE`:
//! `before` asserts the copying path's lower bounds, `after` the shared-cell
//! path's upper bounds (LWW against the pinned `BASE_LWW_*` readings); unset,
//! the test only prints.
//!
//! A local allocation proof, not a CI guard: CI never enables `count-alloc`,
//! and the counters are process-global, so it is meaningful only single-
//! threaded:
//! `cargo test --release -p topgun-server --features count-alloc --test or_write_alloc -- --ignored --test-threads=1 --nocapture`

#![cfg(feature = "count-alloc")]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::alloc::System;
use std::future::Future;
use std::sync::Arc;

use stats_alloc::{StatsAlloc, INSTRUMENTED_SYSTEM};

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_server::storage::datastores::{
    RedbDataStore, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
};
use topgun_server::storage::engines::HashMapStorage;
use topgun_server::storage::impls::{DefaultRecordStore, StorageConfig};
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::merkle_sync::{MerkleMutationObserver, MerkleSyncManager};
use topgun_server::storage::mutation_observer::{CompositeMutationObserver, MutationObserver};
use topgun_server::storage::record::{OrMapEntry, Record, RecordValue};
use topgun_server::storage::record_store::{
    CallerProvenance, ExpiryPolicy, MutateOutcome, RecordStore,
};
use topgun_server::storage::wal::{OrDelta, Wal, WalFsyncPolicy, WalWriter};

#[global_allocator]
static ALLOC: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const MAP: &str = "or_alloc";
const SIZES: [usize; 2] = [1_000, 10_000];
/// Measured ops per reading; odd, so the median is one reading.
const REPS: usize = 9;
/// Measured LWW ops per reading.
const LWW_REPS: usize = 33;
/// Un-measured LWW inserts before the insert reading (see `lww_readings`).
const WARM_KEYS: u64 = 4_000;

/// Leaf-hash slope of the Merkle observer: the only per-entry allocation an
/// in-place OR write is allowed once no whole-record copy is made (16 000 B at
/// N = 1 000 and 160 000 B at N = 10 000).
const LEAF_HASH_SLOPE: f64 = 16.0;

/// LWW readings of the copying path (allocations, bytes), pinned from the
/// `before` run so the `after` run can assert against them.
const BASE_LWW_UPDATE: (u64, u64) = (41, 3_267);
const BASE_LWW_INSERT: (u64, u64) = (40, 3_261);

/// Allowance on the LWW bytes for the write-behind store's boxed
/// `add_with_witness` future, which grows by 24 B (360 → 384) once the write
/// takes a `WriteSource` instead of a `&RecordValue`: same allocation count,
/// one larger block per op.
const LWW_FUTURE_ALLOWANCE: u64 = 32;

#[derive(Clone, Copy, Debug)]
struct Reading {
    allocations: u64,
    bytes: u64,
}

async fn measure<F: Future>(fut: F) -> (F::Output, Reading) {
    let before = INSTRUMENTED_SYSTEM.stats();
    let out = fut.await;
    let after = INSTRUMENTED_SYSTEM.stats();
    (
        out,
        Reading {
            allocations: (after.allocations - before.allocations) as u64,
            bytes: (after.bytes_allocated - before.bytes_allocated) as u64,
        },
    )
}

fn median(mut readings: Vec<Reading>) -> Reading {
    readings.sort_by_key(|r| r.bytes);
    let mid = readings[readings.len() / 2];
    let mut counts: Vec<u64> = readings.iter().map(|r| r.allocations).collect();
    counts.sort_unstable();
    Reading {
        allocations: counts[counts.len() / 2],
        bytes: mid.bytes,
    }
}

/// The smallest reading: the op's own cost with no amortized container growth
/// (a `DashMap` shard, a partition queue) landing in its window. Used for the
/// LWW cases, where a growth step outweighs the one-block difference the
/// readings exist to show.
fn minimum(readings: &[Reading]) -> Reading {
    Reading {
        allocations: readings
            .iter()
            .map(|r| r.allocations)
            .min()
            .expect("readings"),
        bytes: readings.iter().map(|r| r.bytes).min().expect("readings"),
    }
}

fn side() -> Option<String> {
    std::env::var("OR_WRITE_ALLOC_SIDE").ok()
}

fn ts(millis: u64) -> Timestamp {
    Timestamp {
        millis,
        counter: 0,
        node_id: "node-a".to_string(),
    }
}

fn entry(tag: String) -> OrMapEntry {
    OrMapEntry {
        value: Value::Null,
        tag,
        timestamp: ts(1_700_000_000_000),
    }
}

fn or_slot(n: usize) -> RecordValue {
    RecordValue::OrMap {
        records: (0..n).map(|i| entry(format!("{i:020}:0:node-a"))).collect(),
        tombstones: Vec::new(),
    }
}

/// Everything one reading needs: the record store over write-behind over redb,
/// with the WAL armed. The directories live as long as the fixture.
struct Fixture {
    store: DefaultRecordStore,
    inner: Arc<RedbDataStore>,
    _write_behind: Arc<WriteBehindDataStore>,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let inner = Arc::new(RedbDataStore::new(dir.path().join("db.redb")).expect("redb"));
    let wal_dir = dir.path().join("wal");
    let writer = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::None).expect("wal writer");
    let wal: Arc<dyn Wal> = writer;
    let write_behind = WriteBehindDataStore::new_with_wal(
        Arc::clone(&inner) as Arc<dyn MapDataStore>,
        WriteBehindConfig {
            write_delay_ms: 600_000,
            flush_interval_ms: 600_000,
            wal_dir,
            or_delta_wal: true,
            ..WriteBehindConfig::default()
        },
        Some(WalBootstrap {
            wal,
            sequence_start: 1,
        }),
    );
    assert!(
        write_behind.wants_or_witness(),
        "the WAL is armed for delta framing"
    );
    let manager = Arc::new(MerkleSyncManager::default());
    let merkle: Arc<dyn MutationObserver> =
        Arc::new(MerkleMutationObserver::new(manager, MAP.to_string(), 0));
    let store = DefaultRecordStore::new(
        MAP.to_string(),
        0,
        Box::new(HashMapStorage::new()),
        Arc::clone(&write_behind) as Arc<dyn MapDataStore>,
        Arc::new(CompositeMutationObserver::new(vec![merkle])),
        StorageConfig::default(),
    );
    Fixture {
        store,
        inner,
        _write_behind: write_behind,
        _dir: dir,
    }
}

/// Seeds `key` with N entries in redb and loads it, so the slot is resident.
async fn seed_resident(fx: &Fixture, key: &str, n: usize) {
    fx.inner
        .add(MAP, key, &or_slot(n), 0, 0)
        .await
        .expect("seed redb");
    let loaded = fx.store.get(key, false).await.expect("load");
    assert!(loaded.is_some(), "the seeded key loads");
    assert!(fx.store.exists_in_memory(key), "the seeded key is resident");
}

/// One `OR_ADD` of `tag` through `update_in_place`, shaped as the CRDT write
/// path shapes it: the add fold (drop an equal tag, then append unless the tag
/// is tombstoned) and, when the store demands one, the applied entry as the
/// witness.
async fn or_add(store: &DefaultRecordStore, key: &str, tag: &str) -> bool {
    let witness_wanted = store.or_witness_wanted();
    let mut new_entry = Some(entry(tag.to_string()));
    let mut merge_add = move |value: &mut RecordValue| {
        let entry = new_entry.take().expect("the merge runs once");
        let mut witness = None;
        if let RecordValue::OrMap {
            records,
            tombstones,
        } = value
        {
            if !tombstones.iter().any(|t| t == &entry.tag) {
                records.retain(|r| r.tag != entry.tag);
                if witness_wanted {
                    witness = Some(OrDelta::Add {
                        entry: entry.clone(),
                    });
                }
                records.push(entry);
            }
        }
        MutateOutcome {
            changed: true,
            witness,
        }
    };
    store
        .update_in_place(
            key,
            Some(RecordValue::OrMap {
                records: Vec::new(),
                tombstones: Vec::new(),
            }),
            ExpiryPolicy::NONE,
            CallerProvenance::CrdtMerge,
            &mut merge_add,
        )
        .await
        .expect("or_add")
}

fn engine_clone(store: &DefaultRecordStore, key: &str) -> Reading {
    let before = INSTRUMENTED_SYSTEM.stats();
    let clone = store.storage().get(key);
    let after = INSTRUMENTED_SYSTEM.stats();
    assert!(clone.is_some(), "the slot is resident");
    drop(clone);
    Reading {
        allocations: (after.allocations - before.allocations) as u64,
        bytes: (after.bytes_allocated - before.bytes_allocated) as u64,
    }
}

fn slope(a: Reading, b: Reading) -> f64 {
    (b.bytes as f64 - a.bytes as f64) / (SIZES[1] - SIZES[0]) as f64
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
}

async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// Per N: (c, resident `OR_ADD`).
async fn resident_readings() -> Vec<(usize, Reading, Reading)> {
    let fx = fixture();
    // Un-measured warm-up on its own key: first-use allocations (WAL segment,
    // queue, staging and Merkle tree shapes, the flush loop's first tick) land
    // here and in no measured op.
    seed_resident(&fx, "kwarm", SIZES[0]).await;
    for i in 0..3 {
        assert!(or_add(&fx.store, "kwarm", &format!("warm-{i}")).await);
    }
    let mut out = Vec::new();
    for n in SIZES {
        let key = format!("k{n}");
        seed_resident(&fx, &key, n).await;
        // Growth of the slot's own vector and the key's first queue / staging
        // entry happen here, not in a measured op.
        assert!(or_add(&fx.store, &key, "warm").await);
        let c = median((0..REPS).map(|_| engine_clone(&fx.store, &key)).collect());
        let mut ops = Vec::new();
        for i in 0..REPS {
            settle().await;
            let (changed, r) = measure(or_add(&fx.store, &key, &format!("op-{i}"))).await;
            assert!(changed, "the add is applied");
            ops.push(r);
        }
        assert!(fx.store.exists_in_memory(&key), "the slot stayed resident");
        out.push((n, c, median(ops)));
    }
    out
}

/// Per N: (c, materialize `OR_ADD` of an evicted key whose write is pending).
async fn materialize_readings() -> Vec<(usize, Reading, Reading)> {
    let fx = fixture();
    seed_resident(&fx, "kwarm", SIZES[0]).await;
    for i in 0..3 {
        assert!(or_add(&fx.store, "kwarm", &format!("warm-{i}")).await);
        assert!(fx.store.evict_lru(u32::MAX, false) > 0);
        assert!(or_add(&fx.store, "kwarm", &format!("warm-m{i}")).await);
    }
    let mut out = Vec::new();
    for n in SIZES {
        let key = format!("k{n}");
        seed_resident(&fx, &key, n).await;
        assert!(or_add(&fx.store, &key, "warm").await);
        let c = median((0..REPS).map(|_| engine_clone(&fx.store, &key)).collect());
        let mut ops = Vec::new();
        for i in 0..REPS {
            // The key's in-place write is pending (queued and staged) from the
            // previous op; evict it from the engine, then write it again.
            assert!(
                fx.store.evict_lru(u32::MAX, false) > 0,
                "the pending key is evicted"
            );
            assert!(!fx.store.exists_in_memory(&key), "the key is not resident");
            settle().await;
            let (changed, r) = measure(or_add(&fx.store, &key, &format!("op-{i}"))).await;
            assert!(changed, "the add is applied");
            assert!(
                fx.store.exists_in_memory(&key),
                "the op materialized the key"
            );
            ops.push(r);
        }
        out.push((n, c, median(ops)));
    }
    out
}

fn lww(i: u64) -> RecordValue {
    RecordValue::Lww {
        value: Value::Int(i as i64),
        timestamp: ts(1_700_000_000_000 + i),
    }
}

/// (LWW update of a resident key, LWW insert of a new key).
async fn lww_readings() -> (Reading, Reading) {
    let fx = fixture();
    let put = |key: String, i: u64| {
        let store = &fx.store;
        async move {
            store
                .put(&key, lww(i), ExpiryPolicy::NONE, CallerProvenance::Client)
                .await
                .expect("put")
        }
    };
    for i in 0..8 {
        put("lww-resident".to_string(), i).await;
    }
    // The write-behind store seeds each WAL partition on its first write, and
    // that seed reads the partition's un-applied frames; a new key almost always
    // lands in an untouched partition, so without this warm-up every insert
    // reading would carry a seed whose cost grows with the WAL. Enough warm keys
    // to touch every partition keep the seed out of the measured inserts.
    for i in 0..WARM_KEYS {
        put(format!("lww-warm-{i}"), i).await;
    }
    let mut updates = Vec::new();
    for i in 0..LWW_REPS as u64 {
        settle().await;
        let (_, r) = measure(put("lww-resident".to_string(), 100 + i)).await;
        updates.push(r);
    }
    let mut inserts = Vec::new();
    for i in 0..LWW_REPS as u64 {
        // The key is built outside the window: its `String` is the caller's.
        let key = format!("lww-new-{i:04}");
        settle().await;
        let (_, r) = measure(put(key, 100 + i)).await;
        inserts.push(r);
    }
    if std::env::var("OR_WRITE_ALLOC_DUMP").is_ok() {
        println!("lww update reps {updates:?}");
        println!("lww insert reps {inserts:?}");
    }
    if std::env::var("OR_WRITE_ALLOC_DUMP").is_ok() {
        println!("lww update reps {updates:?}");
        println!("lww insert reps {inserts:?}");
    }
    (minimum(&updates), minimum(&inserts))
}

#[test]
#[ignore = "local allocation proof: run single-threaded under count-alloc"]
fn count_alloc_resident_or_add() {
    let readings = runtime().block_on(resident_readings());
    for (n, c, op) in &readings {
        println!(
            "resident N={n} c_bytes={} c_allocs={} op_bytes={} op_allocs={}",
            c.bytes, c.allocations, op.bytes, op.allocations
        );
    }
    let c = slope(readings[0].1, readings[1].1);
    let op = slope(readings[0].2, readings[1].2);
    println!(
        "resident slope_c={c:.3} slope_op={op:.3} ratio={:.3} leaf_hash_slope={LEAF_HASH_SLOPE}",
        op / c
    );
    match side().as_deref() {
        Some("before") => assert!(
            op >= 3.0 * c,
            "copying path: slope {op:.3} B/entry must be at least 3 c = {:.3}",
            3.0 * c
        ),
        Some("after") => assert!(
            op <= 1.1 * LEAF_HASH_SLOPE,
            "shared cell: slope {op:.3} B/entry must be at most {:.3}",
            1.1 * LEAF_HASH_SLOPE
        ),
        _ => {}
    }
}

#[test]
#[ignore = "local allocation proof: run single-threaded under count-alloc"]
fn count_alloc_materialize_or_add() {
    let readings = runtime().block_on(materialize_readings());
    for (n, c, op) in &readings {
        println!(
            "materialize N={n} c_bytes={} op_bytes={} op_allocs={}",
            c.bytes, op.bytes, op.allocations
        );
    }
    let c = slope(readings[0].1, readings[1].1);
    let op = slope(readings[0].2, readings[1].2);
    println!(
        "materialize slope_c={c:.3} slope_op={op:.3} ratio={:.3}",
        op / c
    );
    match side().as_deref() {
        Some("before") => assert!(
            op >= 4.5 * c,
            "copying path: slope {op:.3} B/entry must be at least 4.5 c = {:.3}",
            4.5 * c
        ),
        // A materialize keeps one copy (the `on_load` pre-image) and hashes
        // the slot twice: once for `on_load(pre)`, once for `on_update(post)`.
        Some("after") => assert!(
            op <= 1.1 * (c + 2.0 * LEAF_HASH_SLOPE),
            "shared cell: slope {op:.3} B/entry must be at most 1.1 (c + 32) = {:.3}",
            1.1 * (c + 2.0 * LEAF_HASH_SLOPE)
        ),
        _ => {}
    }
}

/// The heap block a shared slot cell costs a new key: the `Arc` header (two
/// counters) plus a `parking_lot::Mutex<Record>`, rounded up to its alignment.
/// Expressed through `parking_lot` directly so the same expression is valid on
/// a revision without the slot-cell type.
fn cell_block_bytes() -> u64 {
    let cell = std::mem::size_of::<parking_lot::Mutex<Record>>();
    let align = std::mem::align_of::<parking_lot::Mutex<Record>>();
    let raw = 2 * std::mem::size_of::<usize>() + cell;
    raw.div_ceil(align).saturating_mul(align) as u64
}

#[test]
#[ignore = "local allocation proof: run single-threaded under count-alloc"]
fn count_alloc_lww_update_and_insert() {
    let (update, insert) = runtime().block_on(lww_readings());
    let block = cell_block_bytes();
    let record = std::mem::size_of::<Record>() as u64;
    let pointer = std::mem::size_of::<usize>() as u64;
    println!(
        "lww update_allocs={} update_bytes={} insert_allocs={} insert_bytes={}",
        update.allocations, update.bytes, insert.allocations, insert.bytes
    );
    println!(
        "size_of_record={record} cell_block_bytes={block} per_key_resident_overhead={}",
        block as i64 - record as i64 + pointer as i64
    );
    if side().as_deref() == Some("after") {
        assert!(
            update.allocations <= BASE_LWW_UPDATE.0
                && update.bytes <= BASE_LWW_UPDATE.1 + LWW_FUTURE_ALLOWANCE,
            "LWW update: {update:?} must be within the base {BASE_LWW_UPDATE:?} + {LWW_FUTURE_ALLOWANCE} B"
        );
        assert!(
            insert.allocations <= BASE_LWW_INSERT.0 + 1
                && insert.bytes <= BASE_LWW_INSERT.1 + LWW_FUTURE_ALLOWANCE + block,
            "LWW insert: {insert:?} must be within the base {BASE_LWW_INSERT:?} + {LWW_FUTURE_ALLOWANCE} B + one cell block ({block} B)"
        );
    }
}
