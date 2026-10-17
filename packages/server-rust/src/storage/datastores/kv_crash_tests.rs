//! Main tests of the crash suite: every engine, and the map registry above
//! it, recovers from a crash to a prefix of its batches, each whole, holding
//! every durable one (`TG-ENG-001`, `TG-ENG-005`, `TG-ENG-006`).

use std::time::Instant;

use redb::StorageBackend;

use super::kv_crash::{
    crash_case, every_crash_point, random_crashes, CrashMode, CrashTarget, CrashableBackend,
    KvWorkload, MemTarget, RedbTarget, RegistryWorkload, Scenario, CRASH_MODES,
};

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
