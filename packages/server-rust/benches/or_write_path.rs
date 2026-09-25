//! Latency bench of the OR write path through the CRDT service.
//!
//! Every op is a client `OR_ADD` sent through `Service<Operation>` on
//! `Arc<CrdtService>`, over a record-store factory backed by a write-behind
//! store on redb with the WAL armed for delta framing, and the production
//! Merkle observer factory attached. Three cases:
//!
//! * `resident` — `OR_ADD` on a resident slot of N ∈ {1 000, 10 000, 20 000}
//!   entries; the write-behind flush never runs during the case.
//! * `materialize` — the key is evicted from the engine before every op (its
//!   previous write still pending in the write-behind store), so each op
//!   materializes the slot again; the eviction is outside the timed window.
//! * `flush` — `OR_ADD`s on many small keys while the flush loop runs every
//!   5 ms with no write delay. In the `loaded` phase a second task keeps
//!   writing one 20 000-entry key, so every tick flushes that key too; the
//!   `control` phase is the same run without that task. The small keys' p99
//!   is compared across the two phases (the engine's shard is not addressable
//!   from outside, so enough small keys are used that several share the big
//!   key's shard; the largest per-key p99 is printed as well).
//!
//! Each op adds a new unique tag, so a slot grows by one entry per op; the
//! growth over a case is at most `WARMUP + REPS` entries.
//!
//! `cargo bench --bench or_write_path [-- resident|materialize|flush]`

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tower::Service;

use topgun_core::messages::sync::ClientOpMessage;
use topgun_core::{hash_to_partition, ClientOp, ORMapRecord, SystemClock, Timestamp, HLC};
use topgun_server::network::connection::ConnectionRegistry;
use topgun_server::service::domain::crdt::CrdtService;
use topgun_server::service::domain::query::QueryRegistry;
use topgun_server::service::domain::schema::SchemaService;
use topgun_server::service::operation::{service_names, CallerOrigin, Operation, OperationContext};
use topgun_server::service::security::{SecurityConfig, WriteAdmission};
use topgun_server::storage::datastores::{
    RedbDataStore, WalBootstrap, WriteBehindConfig, WriteBehindDataStore,
};
use topgun_server::storage::factory::{ObserverFactory, RecordStoreFactory};
use topgun_server::storage::impls::StorageConfig;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::merkle_sync::{MerkleObserverFactory, MerkleSyncManager};
use topgun_server::storage::record::{OrMapEntry, RecordValue};
use topgun_server::storage::wal::{Wal, WalFsyncPolicy, WalWriter};

const MAP: &str = "or_bench";
const NODE: &str = "bench-node";
const SIZES: [usize; 3] = [1_000, 10_000, 20_000];
const WARMUP: usize = 20;
const RESIDENT_REPS: usize = 200;
const MATERIALIZE_REPS: usize = 100;
const SMALL_KEYS: usize = 256;
const SMALL_SLOT: usize = 10;
const BIG_SLOT: usize = 20_000;
const FLUSH_OPS: usize = 4_000;

struct Node {
    crdt: Arc<CrdtService>,
    factory: Arc<RecordStoreFactory>,
    inner: Arc<RedbDataStore>,
    _write_behind: Arc<WriteBehindDataStore>,
    _dir: tempfile::TempDir,
}

fn node(write_delay_ms: u64, flush_interval_ms: u64) -> Node {
    let dir = tempfile::tempdir().expect("tempdir");
    let inner = Arc::new(RedbDataStore::new(dir.path().join("db.redb")).expect("redb"));
    let wal_dir = dir.path().join("wal");
    let writer = WalWriter::new(wal_dir.clone(), WalFsyncPolicy::None).expect("wal writer");
    let wal: Arc<dyn Wal> = writer;
    let write_behind = WriteBehindDataStore::new_with_wal(
        Arc::clone(&inner) as Arc<dyn MapDataStore>,
        WriteBehindConfig {
            write_delay_ms,
            flush_interval_ms,
            wal_dir,
            or_delta_wal: true,
            ..WriteBehindConfig::default()
        },
        Some(WalBootstrap {
            wal,
            sequence_start: 1,
        }),
    );
    let merkle: Arc<dyn ObserverFactory> = Arc::new(MerkleObserverFactory::new(Arc::new(
        MerkleSyncManager::default(),
    )));
    let factory = Arc::new(
        RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::clone(&write_behind) as Arc<dyn MapDataStore>,
            Vec::new(),
        )
        .with_observer_factories(vec![merkle]),
    );
    let hlc = Arc::new(Mutex::new(HLC::new(
        NODE.to_string(),
        Box::new(SystemClock),
    )));
    let write_validator = Arc::new(WriteAdmission::new(
        Arc::new(SecurityConfig::default()),
        hlc,
    ));
    let crdt = Arc::new(CrdtService::new(
        Arc::clone(&factory),
        Arc::new(ConnectionRegistry::new()),
        write_validator,
        Arc::new(QueryRegistry::new()),
        Arc::new(SchemaService::new()),
    ));
    Node {
        crdt,
        factory,
        inner,
        _write_behind: write_behind,
        _dir: dir,
    }
}

fn ts() -> Timestamp {
    Timestamp {
        millis: 1_700_000_000_000,
        counter: 0,
        node_id: NODE.to_string(),
    }
}

fn or_slot(n: usize) -> RecordValue {
    RecordValue::OrMap {
        records: (0..n)
            .map(|i| OrMapEntry {
                value: topgun_core::types::Value::Null,
                tag: format!("{i:020}:0:{NODE}"),
                timestamp: ts(),
            })
            .collect(),
        tombstones: Vec::new(),
    }
}

/// Seeds `key` with N entries in redb and loads it, so the slot is resident.
async fn seed_resident(node: &Node, key: &str, n: usize) {
    node.inner
        .add(MAP, key, &or_slot(n), 0, 0)
        .await
        .expect("seed redb");
    let store = node.factory.get_or_create(MAP, hash_to_partition(key));
    assert!(
        store.get(key, false).await.expect("load").is_some(),
        "the seeded key loads"
    );
}

fn or_add_op(key: &str, tag: &str) -> ClientOp {
    ClientOp {
        id: Some(format!("{MAP}/{key}/{tag}")),
        map_name: MAP.to_string(),
        key: key.to_string(),
        op_type: None,
        record: None,
        or_record: Some(Some(ORMapRecord {
            value: rmpv::Value::Nil,
            timestamp: ts(),
            tag: tag.to_string(),
            ttl_ms: None,
        })),
        or_tag: Some(Some(tag.to_string())),
        write_concern: None,
        timeout: None,
    }
}

async fn or_add(crdt: &Arc<CrdtService>, key: &str, tag: &str) -> Duration {
    let mut ctx = OperationContext::new(0, service_names::CRDT, ts(), 5_000);
    ctx.partition_id = Some(hash_to_partition(key));
    ctx.caller_origin = CallerOrigin::System;
    let op = Operation::ClientOp {
        ctx,
        payload: ClientOpMessage {
            payload: or_add_op(key, tag),
        },
    };
    let mut svc = Arc::clone(crdt);
    let start = Instant::now();
    Service::call(&mut svc, op).await.expect("OR_ADD acked");
    start.elapsed()
}

fn quantiles(samples: &mut [Duration]) -> (f64, f64) {
    samples.sort_unstable();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    (
        us(samples[samples.len() / 2]),
        us(samples[(samples.len() * 99 / 100).min(samples.len() - 1)]),
    )
}

async fn resident() {
    for n in SIZES {
        let node = node(600_000, 600_000);
        let key = format!("k{n}");
        seed_resident(&node, &key, n).await;
        for i in 0..WARMUP {
            or_add(&node.crdt, &key, &format!("w{i}")).await;
        }
        let mut samples: Vec<Duration> = Vec::with_capacity(RESIDENT_REPS);
        for i in 0..RESIDENT_REPS {
            samples.push(or_add(&node.crdt, &key, &format!("r{i}")).await);
        }
        let (median, p99) = quantiles(&mut samples);
        println!(
            "or_write_path case=resident N={n} reps={RESIDENT_REPS} median_us={median:.1} p99_us={p99:.1}"
        );
    }
}

async fn materialize() {
    for n in SIZES {
        let node = node(600_000, 600_000);
        let key = format!("k{n}");
        seed_resident(&node, &key, n).await;
        let store = node.factory.get_or_create(MAP, hash_to_partition(&key));
        let mut samples: Vec<Duration> = Vec::with_capacity(MATERIALIZE_REPS);
        for i in 0..WARMUP + MATERIALIZE_REPS {
            or_add(&node.crdt, &key, &format!("p{i}")).await;
            assert!(store.evict_lru(u32::MAX, false) > 0, "the key is evicted");
            assert!(!store.exists_in_memory(&key), "the key is not resident");
            let took = or_add(&node.crdt, &key, &format!("m{i}")).await;
            assert!(store.exists_in_memory(&key), "the op materialized the key");
            if i >= WARMUP {
                samples.push(took);
            }
        }
        let (median, p99) = quantiles(&mut samples);
        println!(
            "or_write_path case=materialize N={n} reps={MATERIALIZE_REPS} median_us={median:.1} p99_us={p99:.1}"
        );
    }
}

async fn durable_len(node: &Node, key: &str) -> usize {
    match node.inner.load(MAP, key).await.expect("load") {
        Some(RecordValue::OrMap { records, .. }) => records.len(),
        _ => 0,
    }
}

/// One phase of the flush case: `FLUSH_OPS` `OR_ADD`s round-robin over the small
/// keys, with or without a task that keeps the big key dirty.
async fn flush_phase(loaded: bool) -> (f64, f64, f64, u64) {
    let node = Arc::new(node(0, 5));
    let small: Vec<String> = (0..SMALL_KEYS).map(|i| format!("s{i:03}")).collect();
    for key in &small {
        seed_resident(&node, key, SMALL_SLOT).await;
    }
    seed_resident(&node, "big", BIG_SLOT).await;
    let big_before = durable_len(&node, "big").await;

    let stop = Arc::new(AtomicBool::new(false));
    let big_ops = Arc::new(AtomicU64::new(0));
    let writer = loaded.then(|| {
        let node = Arc::clone(&node);
        let stop = Arc::clone(&stop);
        let big_ops = Arc::clone(&big_ops);
        tokio::spawn(async move {
            let mut i = 0_u64;
            while !stop.load(Ordering::Relaxed) {
                or_add(&node.crdt, "big", &format!("b{i}")).await;
                big_ops.fetch_add(1, Ordering::Relaxed);
                i += 1;
                // Leave room between big-key writes so the flush loop sees the
                // key idle and flushes it, rather than the writer monopolising
                // the key writer.
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
    });

    for (i, key) in small.iter().cycle().take(SMALL_KEYS).enumerate() {
        or_add(&node.crdt, key, &format!("w{i}")).await;
    }
    let mut all: Vec<Duration> = Vec::with_capacity(FLUSH_OPS);
    let mut per_key: Vec<Vec<Duration>> = vec![Vec::new(); SMALL_KEYS];
    for i in 0..FLUSH_OPS {
        let k = i % SMALL_KEYS;
        let took = or_add(&node.crdt, &small[k], &format!("f{i}")).await;
        all.push(took);
        per_key[k].push(took);
    }
    stop.store(true, Ordering::Relaxed);
    if let Some(writer) = writer {
        writer.await.expect("big-key writer");
    }
    // Let the loop flush what is left, so the durable length shows the big key
    // was flushed during the phase.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let big_after = durable_len(&node, "big").await;
    if loaded {
        assert!(
            big_after > big_before,
            "the big key must have been flushed during the loaded phase"
        );
    }

    let (median, p99) = quantiles(&mut all);
    let max_key_p99 = per_key
        .iter_mut()
        .map(|s| quantiles(s).1)
        .fold(0.0_f64, f64::max);
    (median, p99, max_key_p99, big_ops.load(Ordering::Relaxed))
}

async fn flush() {
    let (c_median, c_p99, c_max, _) = flush_phase(false).await;
    println!(
        "or_write_path case=flush phase=control ops={FLUSH_OPS} median_us={c_median:.1} p99_us={c_p99:.1} max_key_p99_us={c_max:.1}"
    );
    let (l_median, l_p99, l_max, big_ops) = flush_phase(true).await;
    println!(
        "or_write_path case=flush phase=loaded ops={FLUSH_OPS} big_ops={big_ops} median_us={l_median:.1} p99_us={l_p99:.1} max_key_p99_us={l_max:.1}"
    );
    println!(
        "or_write_path case=flush p99_ratio={:.3} max_key_p99_ratio={:.3}",
        l_p99 / c_p99,
        l_max / c_max
    );
}

fn main() {
    let filter = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .unwrap_or_default();
    let run = |case: &str| filter.is_empty() || filter == case;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        if run("resident") {
            resident().await;
        }
        if run("materialize") {
            materialize().await;
        }
        if run("flush") {
            flush().await;
        }
    });
}
