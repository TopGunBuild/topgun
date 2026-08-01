//! Executable gate for the soak harness's WAL frame-kind census and the two
//! matrix values its JSON report records.
//!
//! The soak bench (`benches/soak_harness/`) is declared `harness = false`, so a
//! `#[cfg(test)] mod tests` written inside that crate compiles and never runs
//! under `cargo test` (that command runs the bench's `main`, not libtest). A
//! test that cannot execute is a green that measures nothing, so the census and
//! the epoch-width derivation are sited in `report.rs` and re-included here as
//! an integration target, where the standard libtest harness runs them.
//!
//! What is guarded, and why each guard has to exist:
//!
//! - The census must keep `WalOp::OrDelta` frames and full OR-slot snapshot
//!   frames in SEPARATE buckets, for COUNTS and for BYTES. Folded together, an
//!   armed delta emitter and a disarmed one report the same numbers, and any
//!   claim about per-op frame size is read off a mixture. A count-only guard
//!   passes straight over a byte-side merge, so both are asserted.
//! - The two reported matrix fields (`walFsync`, `epochWidth`) must serialize
//!   under the exact keys their consumers read, and must TRACK their input. A
//!   field that reports a constant, or that emits a key nobody reads, is a
//!   refuter that never fires.
//!
//! Fixtures are real frames written through `format::encode` into a real
//! directory and read back through the real decoder, so the census is exercised
//! end to end rather than against a hand-built struct.

// Mirror the soak harness's crate-level pedantic allow list: this test pulls in
// the bench `report` module by `#[path]`, so it must carry the same allows that
// module was written against (the intentional lossy frame-size casts, prose
// like "kill -9" in doc comments). Kept in sync with
// `benches/soak_harness/main.rs`.
#![allow(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::struct_excessive_bools,
    clippy::similar_names,
    clippy::doc_markdown
)]

#[path = "../benches/soak_harness/report.rs"]
#[allow(dead_code)]
mod report;

use std::io::Write;
use std::path::Path;

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_server::storage::record::{OrMapEntry, RecordValue};
use topgun_server::storage::wal::format;
use topgun_server::storage::wal::{OrDelta, WalEntry, WalOp, WalStorePayload};
use topgun_server::tombstone_frontier_impl::DEFAULT_EPOCH_WIDTH;

use report::{
    effective_epoch_width, scan_wal_frame_sizes, ConfirmApplyReport, DiskReport, MemoryReport,
    SoakReport, TombstoneReport,
};

fn stamp(millis: u64) -> Timestamp {
    Timestamp {
        millis,
        counter: 0,
        node_id: "census".to_string(),
    }
}

/// A per-op OR mutation frame: carries one tag and nothing else.
fn or_delta_entry(sequence: u64) -> WalEntry {
    WalEntry {
        map: "soak_or".to_string(),
        key: "k".to_string(),
        op: WalOp::OrDelta {
            delta: OrDelta::Remove {
                tag: format!("t{sequence}"),
            },
        },
        timestamp: None,
        sequence,
    }
}

/// A full OR-slot snapshot frame, deliberately fat: `slot_entries` live records
/// plus the same number of tombstones. Size is what separates it from a delta
/// frame in the byte buckets, so the fixture makes the separation unmissable.
fn or_snapshot_entry(sequence: u64, slot_entries: usize) -> WalEntry {
    let records: Vec<OrMapEntry> = (0..slot_entries)
        .map(|i| OrMapEntry {
            value: Value::String(format!("value-padding-{i:016}")),
            tag: format!("tag-{i:016}"),
            timestamp: stamp(1_000 + i as u64),
        })
        .collect();
    let tombstones: Vec<String> = (0..slot_entries).map(|i| format!("dead-{i:016}")).collect();
    WalEntry {
        map: "soak_or".to_string(),
        key: "k".to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Record(RecordValue::OrMap {
                records,
                tombstones,
            }),
            expiration_time: None,
        },
        timestamp: None,
        sequence,
    }
}

fn lww_entry(sequence: u64) -> WalEntry {
    WalEntry {
        map: "soak_lww".to_string(),
        key: "k".to_string(),
        op: WalOp::Store {
            value: WalStorePayload::Record(RecordValue::Lww {
                value: Value::String("v".to_string()),
                timestamp: stamp(sequence),
            }),
            expiration_time: None,
        },
        timestamp: Some(stamp(sequence)),
        sequence,
    }
}

fn remove_entry(sequence: u64) -> WalEntry {
    WalEntry {
        map: "soak_or".to_string(),
        key: "k".to_string(),
        op: WalOp::Remove,
        timestamp: None,
        sequence,
    }
}

/// Write the entries as one real WAL segment and return each frame's on-disk
/// byte length, in the order written.
fn write_segment(dir: &Path, entries: &[WalEntry]) -> Vec<u64> {
    let mut file = std::fs::File::create(dir.join("000001.log")).expect("create segment");
    let mut sizes = Vec::new();
    for entry in entries {
        let frame = format::encode(entry).expect("encode frame");
        sizes.push(frame.len() as u64);
        file.write_all(&frame).expect("write frame");
    }
    file.flush().expect("flush segment");
    sizes
}

/// Counts must not merge: a census that pushed both OR kinds into one bucket
/// would report the combined total for each kind, and the two fixture counts are
/// deliberately unequal so that collapse cannot be mistaken for a correct
/// answer.
#[test]
fn census_counts_or_delta_and_or_snapshot_frames_separately() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut entries = Vec::new();
    let mut seq = 1;
    for _ in 0..3 {
        entries.push(or_delta_entry(seq));
        seq += 1;
    }
    for _ in 0..5 {
        entries.push(or_snapshot_entry(seq, 4));
        seq += 1;
    }
    entries.push(lww_entry(seq));
    seq += 1;
    entries.push(remove_entry(seq));
    write_segment(dir.path(), &entries);

    let stats = scan_wal_frame_sizes(dir.path()).expect("scan");

    assert_eq!(
        stats.or_delta_frames, 3,
        "delta frames counted on their own"
    );
    assert_eq!(
        stats.or_snapshot_frames, 5,
        "snapshot frames counted on their own"
    );
    // The combined bucket keeps its old meaning: BOTH kinds, which is exactly
    // why it cannot stand in for either one.
    assert_eq!(
        stats.or_frames, 8,
        "combined bucket still covers both kinds"
    );
    assert_ne!(
        stats.or_delta_frames, stats.or_frames,
        "delta count must not be the combined OR count"
    );
    assert_ne!(
        stats.or_snapshot_frames, stats.or_frames,
        "snapshot count must not be the combined OR count"
    );
    assert_eq!(stats.lww_frames, 1);
    assert_eq!(stats.remove_frames, 1);
}

/// Bytes must not merge either. A count-only guard passes over a byte-side
/// merge, and a mean read off the mixture is what makes a working delta emitter
/// look like a broken one, so each kind's total/max/mean is pinned to the frames
/// of that kind ALONE.
#[test]
fn census_buckets_or_delta_and_or_snapshot_bytes_separately() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut entries = Vec::new();
    let mut seq = 1;
    for _ in 0..3 {
        entries.push(or_delta_entry(seq));
        seq += 1;
    }
    // Uneven slot sizes so the snapshot bucket's max is distinguishable from its
    // mean, and both are far above anything a delta frame can reach.
    for slot in [8_usize, 16, 32] {
        entries.push(or_snapshot_entry(seq, slot));
        seq += 1;
    }
    let sizes = write_segment(dir.path(), &entries);
    let (delta_sizes, snapshot_sizes) = sizes.split_at(3);
    let delta_total: u64 = delta_sizes.iter().sum();
    let snapshot_total: u64 = snapshot_sizes.iter().sum();

    let stats = scan_wal_frame_sizes(dir.path()).expect("scan");

    assert_eq!(
        stats.or_delta_bytes_total, delta_total,
        "delta byte total must cover delta frames only"
    );
    assert_eq!(
        stats.or_snapshot_bytes_total, snapshot_total,
        "snapshot byte total must cover snapshot frames only"
    );
    assert_eq!(
        stats.or_delta_bytes_max,
        *delta_sizes.iter().max().expect("delta max"),
    );
    assert_eq!(
        stats.or_snapshot_bytes_max,
        *snapshot_sizes.iter().max().expect("snapshot max"),
    );

    // Means are per kind, not per mixture.
    let expect_delta_mean = delta_total as f64 / delta_sizes.len() as f64;
    let expect_snapshot_mean = snapshot_total as f64 / snapshot_sizes.len() as f64;
    assert!(
        (stats.or_delta_bytes_mean - expect_delta_mean).abs() < 1e-9,
        "delta mean {} != {expect_delta_mean}",
        stats.or_delta_bytes_mean
    );
    assert!(
        (stats.or_snapshot_bytes_mean - expect_snapshot_mean).abs() < 1e-9,
        "snapshot mean {} != {expect_snapshot_mean}",
        stats.or_snapshot_bytes_mean
    );

    // The merge these assertions exist to catch: if the byte buckets were one
    // bucket, each kind would report the combined figure. The fixture makes the
    // two kinds differ by an order of magnitude so no rounding can hide it.
    let combined = delta_total + snapshot_total;
    assert_eq!(
        stats.or_bytes_total, combined,
        "the combined bucket is the mixture, and stays so"
    );
    assert_ne!(stats.or_delta_bytes_total, combined);
    assert_ne!(stats.or_snapshot_bytes_total, combined);
    assert!(
        stats.or_delta_bytes_mean * 4.0 < stats.or_snapshot_bytes_mean,
        "a delta frame must be far smaller than a snapshot frame: delta mean {} vs snapshot mean {}",
        stats.or_delta_bytes_mean,
        stats.or_snapshot_bytes_mean
    );
    assert!(
        stats.or_delta_bytes_max < stats.or_snapshot_bytes_max,
        "the largest delta frame must be smaller than the largest snapshot frame"
    );
}

/// Unset and set are asserted in ONE test fn on purpose: the process environment
/// is global, so a pair of separate tests could interleave and flake. The
/// original value is restored before returning.
#[test]
fn effective_epoch_width_tracks_the_environment() {
    let previous = std::env::var("TOPGUN_EPOCH_WIDTH").ok();

    std::env::remove_var("TOPGUN_EPOCH_WIDTH");
    assert_eq!(
        effective_epoch_width(),
        DEFAULT_EPOCH_WIDTH,
        "an unset width must report the shipped default"
    );

    std::env::set_var("TOPGUN_EPOCH_WIDTH", "37");
    assert_eq!(
        effective_epoch_width(),
        37,
        "a set width must be reported, not the default"
    );
    assert_ne!(
        DEFAULT_EPOCH_WIDTH, 37,
        "the fixture value must differ from the default, or the assertion above is vacuous"
    );

    // A value the server would reject falls back loudly to the default rather
    // than reporting a width nothing applies.
    std::env::set_var("TOPGUN_EPOCH_WIDTH", "0");
    assert_eq!(effective_epoch_width(), DEFAULT_EPOCH_WIDTH);
    std::env::set_var("TOPGUN_EPOCH_WIDTH", "not-a-number");
    assert_eq!(effective_epoch_width(), DEFAULT_EPOCH_WIDTH);

    match previous {
        Some(v) => std::env::set_var("TOPGUN_EPOCH_WIDTH", v),
        None => std::env::remove_var("TOPGUN_EPOCH_WIDTH"),
    }
}

fn sample_report(wal_fsync: &str, epoch_width: u64) -> SoakReport {
    SoakReport {
        mode: "soak".to_string(),
        duration_secs_target: 60,
        duration_secs_actual: 60,
        churn_clients: 1,
        keyspace: 1,
        wal_fsync: wal_fsync.to_string(),
        epoch_width,
        total_writes: 0,
        write_errors: 0,
        reconnects: 0,
        resends: 0,
        steady_checkpoints: 0,
        recovery_checkpoints: 0,
        crashes: 0,
        convergence_failures: Vec::new(),
        recovery_failures: Vec::new(),
        pending_gates: Vec::new(),
        memory: MemoryReport {
            samples: 0,
            first_mb: 0.0,
            peak_mb: 0.0,
            last_mb: 0.0,
            slope_mb_per_hour: 0.0,
            passed: true,
            reason: None,
        },
        // Every value below is DISTINCT and non-default on purpose: a field
        // hard-wired to a constant, or cross-wired to its neighbour, has to
        // show up as a mismatch rather than coincide with the expected value.
        tombstones: TombstoneReport {
            samples: 720,
            first_bytes: 11,
            peak_bytes: 22,
            last_bytes: 33,
            slope_bytes_per_hour: 248_148.9,
            passed: false,
            reason: Some("tombstone slope exceeded".to_string()),
        },
        disk: DiskReport {
            samples: 60,
            first_mb: 1.5,
            peak_mb: 2.5,
            last_mb: 3.5,
            slope_mb_per_hour: 90.75,
            passed: true,
            reason: None,
        },
        confirm_apply: ConfirmApplyReport {
            confirms: 1727,
            last_confirmed_epoch: 113,
            confirm_errors: 51,
        },
        panic_report: None,
        passed: true,
        finished_reason: "duration reached".to_string(),
        timestamp: "1970-01-01T00:00:00Z".to_string(),
    }
}

/// Every verdict that the run's `passed` is hard-ANDed with must reach the JSON
/// report, under the exact keys a consumer reads, carrying its own input.
///
/// This exists because the opposite was true and cost a real diagnosis: a run
/// could persist `passed: false` while the slope that failed it, and the
/// confirm-apply cursor that says whether the gate was even licensed to fire,
/// lived only in the process's stdout. Asserting mere key presence would not
/// catch it -- a field wired to a constant would pass -- so every assertion
/// below is against a distinct value from `sample_report`.
#[test]
fn soak_report_emits_every_hard_anded_verdict_tracking_its_input() {
    let json = serde_json::to_value(sample_report("batched", 1000)).expect("serialize report");

    let t = json
        .get("tombstones")
        .expect("report carries a tombstones section");
    assert_eq!(
        t.get("samples").and_then(serde_json::Value::as_u64),
        Some(720)
    );
    assert_eq!(
        t.get("firstBytes").and_then(serde_json::Value::as_u64),
        Some(11)
    );
    assert_eq!(
        t.get("peakBytes").and_then(serde_json::Value::as_u64),
        Some(22)
    );
    assert_eq!(
        t.get("lastBytes").and_then(serde_json::Value::as_u64),
        Some(33)
    );
    assert_eq!(
        t.get("slopeBytesPerHour")
            .and_then(serde_json::Value::as_f64),
        Some(248_148.9)
    );
    // The FAILING verdict and its reason are the two the console used to own
    // exclusively, and are the whole point of persisting this section.
    assert_eq!(
        t.get("passed").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        t.get("reason").and_then(serde_json::Value::as_str),
        Some("tombstone slope exceeded")
    );

    let d = json.get("disk").expect("report carries a disk section");
    assert_eq!(
        d.get("samples").and_then(serde_json::Value::as_u64),
        Some(60)
    );
    assert_eq!(
        d.get("slopeMbPerHour").and_then(serde_json::Value::as_f64),
        Some(90.75)
    );
    assert_eq!(
        d.get("passed").and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let c = json
        .get("confirmApply")
        .expect("report carries a confirmApply section");
    assert_eq!(
        c.get("confirms").and_then(serde_json::Value::as_u64),
        Some(1727)
    );
    assert_eq!(
        c.get("lastConfirmedEpoch")
            .and_then(serde_json::Value::as_u64),
        Some(113)
    );
    assert_eq!(
        c.get("confirmErrors").and_then(serde_json::Value::as_u64),
        Some(51)
    );
}

/// The two matrix fields must land in the JSON under the exact keys their
/// consumers read, AND must track their input. Asserting only that the keys
/// exist would pass on a field hard-wired to a constant.
#[test]
fn soak_report_emits_wal_fsync_and_epoch_width_tracking_their_input() {
    let json = serde_json::to_value(sample_report("batched", 1000)).expect("serialize report");

    assert_eq!(
        json.get("walFsync").and_then(serde_json::Value::as_str),
        Some("batched"),
        "walFsync must be present under that exact key and carry the configured policy"
    );
    assert_eq!(
        json.get("epochWidth").and_then(serde_json::Value::as_u64),
        Some(1000),
        "epochWidth must be present under that exact key and carry the effective width"
    );

    // Same report shape, different inputs: the keys must follow the inputs.
    let other = serde_json::to_value(sample_report("per_op", 100)).expect("serialize report");
    assert_eq!(
        other.get("walFsync").and_then(serde_json::Value::as_str),
        Some("per_op")
    );
    assert_eq!(
        other.get("epochWidth").and_then(serde_json::Value::as_u64),
        Some(100)
    );

    // snake_case keys would be silently ignored by every consumer that reads the
    // camelCase names, so their absence is part of the contract.
    assert!(json.get("wal_fsync").is_none());
    assert!(json.get("epoch_width").is_none());
}
