//! Behavioural check: an OR-Map key written before a restart keeps every entry
//! when the FIRST op after the restart is a write rather than a read.
//!
//! After a restart nothing re-hydrates the in-memory engine, so every existing
//! key is durable but not resident. A write that lands on such a key must be
//! absorbed into the key's durable state; if it instead starts from an empty
//! slot, the write-behind flush replaces the durable row and every earlier entry
//! of the key is gone.
//!
//!   add N values  →  kill -9, relaunch (WAL recovery into redb)
//!   add ONE value with no read before it  →  wait for the write-behind flush
//!   kill -9, relaunch  →  read the key: all N + 1 values must be present
//!
//! The control runs the same sequence with a read before the post-restart write,
//! which hydrates the key first.
//!
//! The out-of-process supervisor and the WebSocket client are the soak
//! harness's own, pulled in by `#[path]` exactly as `soak_tombstone_restart.rs`
//! does.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::similar_names,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::struct_excessive_bools
)]

#[path = "../benches/soak_harness/or_noloss.rs"]
#[allow(dead_code)]
mod or_noloss;

#[path = "../benches/soak_harness/client.rs"]
#[allow(dead_code)]
mod client;

#[path = "../benches/soak_harness/process.rs"]
#[allow(dead_code)]
mod process;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use client::SoakClient;
use process::{resolve_server_binary, ServerConfig, ServerSupervisor};

const JWT_SECRET: &str = "test-e2e-secret";
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const OR_MAP: &str = "nonres_or_map";
const OR_KEY: &str = "doc";
/// Values added before the first restart.
const N_BEFORE: i64 = 5;
/// The value added after the first restart.
const AFTER_VALUE: i64 = 99;
/// Longer than the write-behind delay plus one flush tick (1000 + 1000 ms by
/// default), so the post-restart write has reached redb before the second kill.
const FLUSH_WAIT: Duration = Duration::from_secs(4);

fn expected_values() -> HashSet<String> {
    (0..N_BEFORE)
        .chain(std::iter::once(AFTER_VALUE))
        .map(|v| or_noloss::render_value(&rmpv::Value::from(v)))
        .collect()
}

async fn run_scenario(read_before_write: bool) -> HashSet<String> {
    let data_dir = tempfile::tempdir().expect("create temp data dir");
    let port = ServerSupervisor::pick_free_port().expect("pick free port");
    let supervisor: Arc<ServerSupervisor> = ServerSupervisor::new(ServerConfig {
        binary: resolve_server_binary(),
        data_dir: data_dir.path().to_path_buf(),
        port,
        jwt_secret: JWT_SECRET.to_string(),
        wal_fsync_policy: "per_op".to_string(),
    });
    supervisor.start(READY_TIMEOUT).await.expect("server start");

    let mut c = SoakClient::connect(supervisor.addr(), 0, JWT_SECRET)
        .await
        .expect("client connect");
    for v in 0..N_BEFORE {
        c.or_add(OR_MAP, OR_KEY, &format!("t-{v}"), v, 1, v as u32)
            .await
            .expect("or_add before restart");
    }
    let before = c.ormap_read_all(OR_MAP).await.expect("read before restart");
    assert_eq!(
        before.get(OR_KEY).map(HashSet::len),
        Some(N_BEFORE as usize),
        "precondition: all {N_BEFORE} values are visible before the restart"
    );
    drop(c);

    supervisor
        .restart(READY_TIMEOUT)
        .await
        .expect("first restart");

    let mut c = SoakClient::connect(supervisor.addr(), 0, JWT_SECRET)
        .await
        .expect("client connect after first restart");
    if read_before_write {
        let hydrated = c
            .ormap_read_all(OR_MAP)
            .await
            .expect("read after first restart");
        assert_eq!(
            hydrated.get(OR_KEY).map(HashSet::len),
            Some(N_BEFORE as usize),
            "control precondition: recovery kept all {N_BEFORE} values"
        );
    }
    c.or_add(OR_MAP, OR_KEY, "t-after", AFTER_VALUE, 2, 1)
        .await
        .expect("or_add after restart");
    drop(c);

    tokio::time::sleep(FLUSH_WAIT).await;
    supervisor
        .restart(READY_TIMEOUT)
        .await
        .expect("second restart");

    let mut c = SoakClient::connect(supervisor.addr(), 0, JWT_SECRET)
        .await
        .expect("client connect after second restart");
    let after = c
        .ormap_read_all(OR_MAP)
        .await
        .expect("read after second restart");
    drop(c);
    supervisor.shutdown().await;

    after.get(OR_KEY).cloned().unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn first_post_restart_or_add_keeps_every_earlier_value() {
    let observed = run_scenario(false).await;
    let expected = expected_values();
    let mut missing: Vec<&String> = expected.difference(&observed).collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "values lost across restart + write-without-read: missing={missing:?} observed={observed:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn post_restart_or_add_after_a_read_keeps_every_earlier_value_control() {
    let observed = run_scenario(true).await;
    let expected = expected_values();
    let mut missing: Vec<&String> = expected.difference(&observed).collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "control (read before write): missing={missing:?} observed={observed:?}"
    );
}

/// Repetitions per entry count in the cost reading.
const COST_REPETITIONS: usize = 20;

fn median_and_max(samples: &mut [Duration]) -> (Duration, Duration) {
    samples.sort();
    (samples[samples.len() / 2], samples[samples.len() - 1])
}

/// Cost reading, not a gate: the first OR write on a key after a restart now
/// loads the key's durable value before it mutates it. Measures the OP_ACK
/// latency of that first write against the write right after it (the key is
/// then resident), for keys of 1 000 and 10 000 entries, over
/// `COST_REPETITIONS` restarts each, and prints median and max. Run with
/// `--ignored --nocapture`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "cost reading; run explicitly with --ignored --nocapture"]
async fn cost_of_the_first_post_restart_or_add() {
    for entries in [1_000_i64, 10_000] {
        let data_dir = tempfile::tempdir().expect("create temp data dir");
        let port = ServerSupervisor::pick_free_port().expect("pick free port");
        let supervisor: Arc<ServerSupervisor> = ServerSupervisor::new(ServerConfig {
            binary: resolve_server_binary(),
            data_dir: data_dir.path().to_path_buf(),
            port,
            jwt_secret: JWT_SECRET.to_string(),
            wal_fsync_policy: "per_op".to_string(),
        });
        supervisor.start(READY_TIMEOUT).await.expect("server start");

        let mut c = SoakClient::connect(supervisor.addr(), 0, JWT_SECRET)
            .await
            .expect("client connect");
        for v in 0..entries {
            c.or_add(OR_MAP, OR_KEY, &format!("t-{v}"), v, 1, v as u32)
                .await
                .expect("seed or_add");
        }
        drop(c);
        tokio::time::sleep(FLUSH_WAIT).await;

        let mut first = Vec::with_capacity(COST_REPETITIONS);
        let mut next = Vec::with_capacity(COST_REPETITIONS);
        for rep in 0..COST_REPETITIONS {
            supervisor.restart(READY_TIMEOUT).await.expect("restart");
            let mut c = SoakClient::connect(supervisor.addr(), 0, JWT_SECRET)
                .await
                .expect("client connect after restart");
            let started = std::time::Instant::now();
            c.or_add(OR_MAP, OR_KEY, &format!("t-first-{rep}"), -1, 2, rep as u32)
                .await
                .expect("first post-restart or_add");
            first.push(started.elapsed());
            let started = std::time::Instant::now();
            c.or_add(OR_MAP, OR_KEY, &format!("t-next-{rep}"), -2, 3, rep as u32)
                .await
                .expect("next or_add");
            next.push(started.elapsed());
            drop(c);
            // Let the two writes reach redb so the next restart replays nothing.
            tokio::time::sleep(FLUSH_WAIT).await;
        }
        supervisor.shutdown().await;

        let (first_median, first_max) = median_and_max(&mut first);
        let (next_median, next_max) = median_and_max(&mut next);
        println!(
            "COST entries={entries} reps={COST_REPETITIONS} \
             first_median_us={} first_max_us={} next_median_us={} next_max_us={}",
            first_median.as_micros(),
            first_max.as_micros(),
            next_median.as_micros(),
            next_max.as_micros()
        );
    }
}
