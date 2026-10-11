//! Guard: the redb store keeps no memory per distinct map name.
//!
//! A read of a map that was never written reaches the table definition and
//! finds no table, so nothing is created on disk. Whatever the process still
//! holds after many such reads, each under a different name, is memory retained
//! per name — a server that reads (or syncs) many short-lived map names would
//! grow by that much for its whole lifetime.
//!
//! Two arms, each `CALLS` calls of `MapDataStore::load`:
//!
//! * distinct — every call uses a different map name; growth `D`.
//! * same — every call uses one name; growth `C`, the control that shows the
//!   call itself retains nothing.
//!
//! Growth is live bytes (allocated minus freed) after the arm minus before it,
//! read from a counting global allocator. The counters are process-global, so
//! this binary holds exactly one test, on a current-thread runtime, and reads
//! them only after the store, the runtime and a warm-up call exist.
//!
//! Names are outside the identifier class and longer than 200 bytes, so a
//! retained copy of a name is far larger than anything else a call could leave
//! behind: one retained copy per name is over a megabyte across the arm.

#![allow(clippy::cast_possible_wrap)]

use std::alloc::System;

use stats_alloc::{StatsAlloc, INSTRUMENTED_SYSTEM};

use topgun_server::storage::datastores::RedbDataStore;
use topgun_server::storage::map_data_store::MapDataStore;

#[global_allocator]
static ALLOC: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const CALLS: usize = 5_000;
const NAME_PREFIX_LEN: usize = 200;
const LIMIT_BYTES: i64 = 64 * 1024;

fn live_bytes() -> i64 {
    let stats = INSTRUMENTED_SYSTEM.stats();
    stats.bytes_allocated as i64 - stats.bytes_deallocated as i64
}

fn map_name(prefix: &str, i: usize) -> String {
    format!("{prefix}/{i}")
}

async fn read_absent(store: &RedbDataStore, map: &str) {
    let loaded = store.load(map, "k").await.expect("load");
    assert!(loaded.is_none(), "the map was never written");
}

#[test]
fn reads_of_distinct_map_names_retain_no_memory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RedbDataStore::new(dir.path().join("db.redb")).expect("redb");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");
    let prefix = "d".repeat(NAME_PREFIX_LEN);
    let same_name = format!("{}/same", "s".repeat(NAME_PREFIX_LEN));

    let (distinct, same) = runtime.block_on(async {
        // First-use allocations of the store and the runtime land here.
        read_absent(&store, &map_name(&prefix, CALLS)).await;

        let before = live_bytes();
        for i in 0..CALLS {
            read_absent(&store, &map_name(&prefix, i)).await;
        }
        let distinct = live_bytes() - before;

        let before = live_bytes();
        for _ in 0..CALLS {
            read_absent(&store, &same_name).await;
        }
        let same = live_bytes() - before;

        (distinct, same)
    });

    println!("redb_table_name_alloc calls={CALLS} D={distinct} C={same} limit={LIMIT_BYTES}");
    assert!(
        distinct < LIMIT_BYTES,
        "{CALLS} reads of distinct map names retained {distinct} B (limit {LIMIT_BYTES} B)"
    );
    assert!(
        same < LIMIT_BYTES,
        "{CALLS} reads of one map name retained {same} B (limit {LIMIT_BYTES} B)"
    );
}
