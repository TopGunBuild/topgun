//! Main tests of the crash suite: every engine, and the map registry above
//! it, recovers from a crash to a prefix of its batches, each whole, holding
//! every durable one (`TG-ENG-001`, `TG-ENG-005`, `TG-ENG-006`).

use std::sync::Arc;
use std::time::Instant;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use redb::StorageBackend;

use super::engine_store::KS_PRIMARY;
use super::kv_crash::{
    crash_case, every_crash_point, random_crashes, CrashMode, CrashTarget, CrashableBackend,
    KvWorkload, MemTarget, RedbTarget, RegistryWorkload, Scenario, Step, CRASH_MODES,
};
use super::kv_engine::BatchOp;
use super::redb_engine::RedbKvEngine;

const MIB: usize = 1024 * 1024;

fn pattern(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 241).unwrap_or(0) ^ salt)
        .collect()
}

fn image_bytes(backend: &CrashableBackend) -> Vec<u8> {
    let len = backend.len().expect("len of a live image");
    backend
        .read(0, usize::try_from(len).expect("len fits"))
        .expect("read of a live image")
}

fn assert_dead(backend: &CrashableBackend) {
    assert!(backend.len().is_err(), "len on a dead backend");
    assert!(backend.read(0, 1).is_err(), "read on a dead backend");
    assert!(backend.write(0, &[1]).is_err(), "write on a dead backend");
    assert!(backend.set_len(1).is_err(), "set_len on a dead backend");
    assert!(backend.sync_data(false).is_err(), "sync on a dead backend");
}

#[test]
fn k01_the_crashable_backend_keeps_synced_and_drops_unsynced_writes() {
    let synced = pattern(MIB, 0x5A);
    let unsynced = pattern(8 * 512, 0xA5);
    let build = || {
        let backend = CrashableBackend::new();
        backend.set_len(MIB as u64).expect("set_len");
        backend.write(0, &synced).expect("write");
        backend.sync_data(false).expect("sync");
        backend
            .set_len((MIB + unsynced.len()) as u64)
            .expect("set_len");
        backend.write(MIB as u64, &unsynced).expect("write");
        backend
    };

    // Counted ops: write, set_len and sync_data, once each.
    let backend = build();
    assert_eq!(backend.ops_so_far(), 5);
    assert_eq!(image_bytes(&backend).len(), MIB + unsynced.len());

    // DropUnsynced keeps the synced bytes and nothing after them; the old
    // handle is released and dead through every clone.
    let clone = backend.clone();
    assert!(backend.retained_bytes() >= MIB);
    let image = backend.crash_image(CrashMode::DropUnsynced, 0);
    assert_eq!(backend.retained_bytes(), 0);
    assert_eq!(clone.retained_bytes(), 0);
    assert_dead(&backend);
    assert_dead(&clone);
    assert_eq!(image.ops_so_far(), 0);
    assert_eq!(image_bytes(&image), synced);

    // The keeping modes keep the synced bytes in every image, keep the
    // unsynced write in some and drop it in others; sectors tear it.
    for mode in [CrashMode::KeepRandomWrites, CrashMode::KeepRandomSectors] {
        let mut kept_whole = 0;
        let mut dropped = 0;
        let mut torn = 0;
        for seed in 0..64 {
            let backend = build();
            let image = backend.crash_image(mode, seed);
            assert_eq!(backend.retained_bytes(), 0);
            assert_dead(&backend);
            let bytes = image_bytes(&image);
            assert_eq!(&bytes[..MIB], synced.as_slice(), "{mode:?} seed {seed}");
            // A write kept past a lost set_len does not lengthen the image.
            assert!(
                bytes.len() == MIB || bytes.len() == MIB + unsynced.len(),
                "{mode:?} seed {seed}: length {} is not one a set_len gave",
                bytes.len()
            );
            let tail = bytes.get(MIB..).unwrap_or(&[]);
            let sectors: Vec<bool> = unsynced
                .chunks(512)
                .enumerate()
                .map(|(index, sector)| tail.get(index * 512..(index + 1) * 512) == Some(sector))
                .collect();
            if sectors.iter().all(|&kept| kept) {
                kept_whole += 1;
            } else if sectors.iter().any(|&kept| kept) {
                torn += 1;
            } else {
                dropped += 1;
            }
        }
        assert!(
            kept_whole > 0 || torn > 0,
            "{mode:?} never keeps an unsynced write"
        );
        assert!(dropped > 0, "{mode:?} never drops an unsynced write");
        if mode == CrashMode::KeepRandomSectors {
            assert!(torn > 0, "sectors never tear a write");
        } else {
            assert_eq!(torn, 0, "whole writes tore");
        }
    }

    // Death: the op with the armed index and everything after it fail and
    // apply nothing; an index already passed kills the next op.
    let backend = CrashableBackend::new();
    backend.set_len(1024).expect("set_len");
    backend.write(0, &synced[..1024]).expect("write");
    backend.set_die_at(Some(2));
    assert!(backend.sync_data(false).is_err(), "the armed op dies");
    assert_eq!(backend.ops_so_far(), 3);
    assert_dead(&backend);
    let image = backend.crash_image(CrashMode::DropUnsynced, 7);
    assert_eq!(image_bytes(&image), Vec::<u8>::new(), "nothing was synced");

    let backend = CrashableBackend::new();
    backend.set_len(512).expect("set_len");
    backend.sync_data(false).expect("sync");
    backend.set_die_at(Some(0));
    assert!(
        backend.write(0, &[9; 512]).is_err(),
        "a passed index kills the next op"
    );
    let image = backend.crash_image(CrashMode::KeepRandomWrites, 3);
    assert_eq!(image_bytes(&image), vec![0; 512]);

    let backend = CrashableBackend::new();
    backend.set_die_at(Some(0));
    assert!(backend.set_len(1).is_err(), "die_at 0 kills the first op");
    assert_dead(&backend);
}

#[test]
fn k02_the_mem_crash_image_holds_a_prefix_of_whole_batches() {
    let workload = KvWorkload::fixed();
    let units = workload.units();
    for keep in 0..=units as u64 {
        let outcome = crash_case::<MemTarget, _>(&workload, None, CrashMode::DropUnsynced, keep);
        assert_eq!(outcome.durable, 11);
        let expected = (outcome.durable + usize::try_from(keep).expect("keep fits")).min(units);
        assert!(
            outcome.matches.contains(&expected),
            "keep {keep}: matches {:?}, expected {expected}",
            outcome.matches
        );
    }
    // A death before the trailing batches: the image is the durable prefix
    // plus whole kept batches, never part of one.
    let report = every_crash_point::<MemTarget, _>(&workload);
    assert!(report.cases > 0 && report.lossy > 0 && report.beyond_durable > 0);
}

#[test]
fn k03_redb_every_crash_point_recovers_a_prefix_that_holds_every_durable_batch() {
    let started = Instant::now();
    let workload = KvWorkload::fixed();
    let report = every_crash_point::<RedbTarget, _>(&workload);
    assert_eq!(report.cases, (report.ops as usize + 1) * CRASH_MODES.len());
    assert!(report.lossy > 0, "no crash point lost a batch");

    // Never armed: no death fired and no destructor reached the image, so
    // nothing after the last sync is durable.
    let outcome = crash_case::<RedbTarget, _>(&workload, None, CrashMode::DropUnsynced, 0);
    assert_eq!(outcome.durable, 11);
    assert_eq!(outcome.matches, vec![11], "B12 must be absent");
    eprintln!(
        "k03: T = {}, {} cases, {} lossy, {:?}",
        report.ops,
        report.cases,
        report.lossy,
        started.elapsed()
    );
}

#[test]
fn k04_redb_random_crashes_recover_a_prefix_that_holds_every_durable_batch() {
    let report = random_crashes::<RedbTarget, _>(0..200, true, KvWorkload::random);
    assert_eq!(report.cases, 200 * CRASH_MODES.len());
    assert!(report.lossy > 0 && report.beyond_durable > 0);
}

#[test]
#[ignore = "10 000 seeds; run with --ignored"]
fn k04b_redb_random_crashes_long() {
    let started = Instant::now();
    let report = random_crashes::<RedbTarget, _>(10_000..20_000, true, KvWorkload::random);
    assert_eq!(report.cases, 10_000 * CRASH_MODES.len());
    eprintln!(
        "k04b: seeds 10000..20000, {} cases, {} lossy, {:?}",
        report.cases,
        report.lossy,
        started.elapsed()
    );
}

#[test]
fn k05_mem_crashes_recover_a_prefix_that_holds_every_durable_batch() {
    let report = every_crash_point::<MemTarget, _>(&KvWorkload::fixed());
    assert!(report.cases > 0 && report.lossy > 0);
    let report = random_crashes::<MemTarget, _>(0..200, true, KvWorkload::random);
    assert_eq!(report.cases, 200);
    assert!(report.lossy > 0);
}

fn registry_suite<T: CrashTarget>() -> usize {
    let mut cases = 0;
    for workload in [
        RegistryWorkload::fixed(),
        RegistryWorkload::fixed_migration(),
    ] {
        let report = every_crash_point::<T, _>(&workload);
        assert!(
            report.lossy > 0,
            "{}: no crash point lost an operation",
            T::NAME
        );
        cases += report.cases;
    }
    cases
}

#[test]
fn k06_redb_registry_after_any_crash_has_no_orphan_and_lists_exactly_the_recovered_maps() {
    let started = Instant::now();
    let exhaustive = registry_suite::<RedbTarget>();
    for migration in [false, true] {
        let report = random_crashes::<RedbTarget, _>(0..200, false, |seed| {
            RegistryWorkload::random(seed, migration)
        });
        assert_eq!(report.cases, 200);
        assert!(report.lossy > 0);
    }
    eprintln!(
        "k06: {exhaustive} exhaustive cases, {:?}",
        started.elapsed()
    );
}

#[test]
fn k07_mem_registry_after_any_crash_has_no_orphan_and_lists_exactly_the_recovered_maps() {
    assert!(registry_suite::<MemTarget>() > 0);
}

/// Batches, rows per batch and value bytes of the growing workload. It is the
/// only shape shown to cross a growth of the redb file while unsynced writes
/// are pending; smaller workloads stay inside the file redb creates.
const GROWING_BATCHES: u32 = 4;
const GROWING_ROWS: u32 = 10_000;
const GROWING_VALUE_BYTES: usize = 60;

/// `k08`'s stride over the crash points: the smallest multiple of 7 that keeps
/// the test within 60 s in a debug build and still fails it without the redb
/// fix that syncs the file length after a growth. Not in every run: redb
/// iterates hash sets in a per-process random order, so which crash images
/// panic varies (24 of 25 runs failed).
const K08_STRIDE: u64 = 161;

/// `GROWING_BATCHES` durable batches of `GROWING_ROWS` primary keys with
/// seeded random values.
fn growing_workload() -> KvWorkload {
    let mut rng = StdRng::seed_from_u64(887);
    let steps = (0..GROWING_BATCHES)
        .map(|batch| {
            let ops = (0..GROWING_ROWS)
                .map(|row| {
                    let mut key = vec![KS_PRIMARY];
                    key.extend_from_slice(&(batch * GROWING_ROWS + row).to_be_bytes());
                    let mut value = vec![0u8; GROWING_VALUE_BYTES];
                    rng.fill(value.as_mut_slice());
                    BatchOp::Put { key, value }
                })
                .collect();
            Step::Batch { ops, sync: true }
        })
        .collect();
    KvWorkload { steps }
}

/// Crashes the growing workload at every `stride`-th op of its dry run, in
/// every mode with seed 0, and returns the number of cases. Each case panics
/// unless the image opens and holds a prefix of whole batches that contains
/// every durable one.
fn growing_crash_sweep(stride: u64) -> usize {
    let workload = growing_workload();

    // The dry run, one batch at a time over a kept handle, so the file length
    // after each batch is visible: the workload must grow the file after its
    // first durable batch, or it crosses no growth a crash could interrupt.
    let backend = CrashableBackend::new();
    let engine = Arc::new(
        RedbKvEngine::with_backend(backend.clone()).expect("redb over a fresh crashable backend"),
    );
    let mut lengths = Vec::new();
    for step in &workload.steps {
        let run = KvWorkload {
            steps: vec![step.clone()],
        }
        .run(&engine);
        assert!(run.ok.iter().all(|&ok| ok), "a dry-run batch failed");
        lengths.push(backend.len().expect("len of the live backend"));
    }
    assert!(
        lengths.windows(2).any(|pair| pair[1] > pair[0]),
        "the file never grows after the first durable batch: lengths {lengths:?}"
    );
    let ops = backend.ops_so_far();
    drop(engine);

    let mut cases = 0;
    for point in (0..ops).step_by(usize::try_from(stride).expect("stride fits")) {
        for mode in CRASH_MODES {
            crash_case::<RedbTarget, _>(&workload, Some(point), mode, 0);
            cases += 1;
        }
    }
    let points = usize::try_from(ops.div_ceil(stride)).expect("points fit");
    assert_eq!(cases, points * CRASH_MODES.len());
    cases
}

#[test]
fn k08_redb_crashes_while_the_file_grows_recover_a_prefix_that_holds_every_durable_batch() {
    let started = Instant::now();
    let cases = growing_crash_sweep(K08_STRIDE);
    eprintln!(
        "k08: stride {K08_STRIDE}, {cases} cases, {:?}",
        started.elapsed()
    );
}

#[test]
#[ignore = "every 7th op of the growing workload; run in release with --ignored"]
fn k09_redb_every_seventh_op_of_a_growing_workload_recovers_a_prefix_that_holds_every_durable_batch(
) {
    let started = Instant::now();
    let cases = growing_crash_sweep(7);
    eprintln!("k09: stride 7, {cases} cases, {:?}", started.elapsed());
}
