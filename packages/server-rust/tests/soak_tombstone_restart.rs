//! AC1 behavioral proof: the OR-Map tombstone-bytes gauge survives `kill -9`.
//!
//! The gauge (`topgun_ormap_tombstone_bytes_total`, scraped from the server's
//! own `GET /metrics`) is a process-local counter that resets to 0 on every
//! process start and never re-counts rehydrated (redb-persisted) tombstones.
//! The startup reconciliation walks the recovered OR-Map keyspace once, after
//! WAL recovery and before the listener opens, and boot-seeds the gauge to the
//! persisted tombstone total. This test proves that end-to-end against a REAL
//! out-of-process server on a REAL redb + WAL:
//!
//!   seed N unique-tag OR removes  →  pre-kill gauge == T (> 0)
//!   kill -9  →  relaunch against the same data dir
//!   FIRST post-restart scrape == T  (not 0)
//!
//! The load-bearing sub-assertion (guards the Prometheus dead-sink failure mode
//! directly): the first post-restart scrape reads T with NO intervening
//! post-restart `add_tombstone_bytes` — reconciliation ALONE, not a subsequent
//! write, must produce the scraped value. So the test issues zero OR mutations
//! between relaunch and the scrape. Code inspection is not a substitute.
//!
//! The out-of-process supervisor (`kill -9` + relaunch against the same
//! `data_dir`) and the WebSocket client are reused verbatim from the soak
//! harness (`benches/soak_harness/{process,client,or_noloss}.rs`), pulled in by
//! `#[path]` so this exercises the exact bench source. The bench is declared
//! `harness = false`, so its own `#[test]`s never run under `cargo test`;
//! re-including the modules here puts this proof under the standard libtest
//! harness. `allow(dead_code)` covers bench items this test does not reference.

#[path = "../benches/soak_harness/or_noloss.rs"]
#[allow(dead_code)]
mod or_noloss;

#[path = "../benches/soak_harness/client.rs"]
#[allow(dead_code)]
mod client;

#[path = "../benches/soak_harness/process.rs"]
#[allow(dead_code)]
mod process;

use std::time::Duration;

use client::SoakClient;
use process::{resolve_server_binary, ServerConfig, ServerSupervisor};

/// JWT secret shared with `SoakClient` (mirrors the soak harness default).
const JWT_SECRET: &str = "test-e2e-secret";
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const OR_MAP: &str = "ork_map";
const OR_KEY: &str = "ork";
/// Number of unique-tag OR removes to seed.
const N_TOMBS: usize = 50;

/// Scrape `topgun_ormap_tombstone_bytes_total` from the running server's
/// `GET /metrics`. Returns `None` on any transient failure (connection refused
/// mid-restart, non-2xx, unreadable body, or the metric line simply absent) —
/// mirroring the soak harness `scrape_tombstone_bytes` contract, so an absent
/// line is treated as "no sample" rather than a spurious 0.
async fn scrape_tombstone_bytes(port: u16) -> Option<u64> {
    let url = format!("http://127.0.0.1:{port}/metrics");
    let resp = reqwest::Client::new().get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    parse_tombstone_bytes_total(&body)
}

/// Parse the `topgun_ormap_tombstone_bytes_total` sample value out of a
/// Prometheus text exposition body.
fn parse_tombstone_bytes_total(body: &str) -> Option<u64> {
    const METRIC: &str = "topgun_ormap_tombstone_bytes_total";
    let labelled_prefix = format!("{METRIC}{{");
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        if name == METRIC || name.starts_with(&labelled_prefix) {
            let value = parts.next()?;
            return value
                .parse::<u64>()
                .ok()
                .or_else(|| value.parse::<f64>().ok().map(|f| f as u64));
        }
    }
    None
}

/// Seed N unique-tag OR removes, scrape the gauge (T > 0), `kill -9`, relaunch,
/// then assert the FIRST post-restart scrape reads T with no intervening write.
///
/// `per_op` WAL fsync makes each acked remove durable in the WAL before its
/// `OP_ACK`, so recovery deterministically replays every tombstone into the
/// inner store — which the boot reconciliation then walks. No sleep-for-flush
/// race: durability is established at ack time, not by the write-behind timer.
#[tokio::test(flavor = "multi_thread")]
async fn tombstone_bytes_gauge_survives_kill9() {
    let data_dir = tempfile::tempdir().expect("create temp data dir");
    let port = ServerSupervisor::pick_free_port().expect("pick free port");

    let supervisor = ServerSupervisor::new(ServerConfig {
        binary: resolve_server_binary(),
        data_dir: data_dir.path().to_path_buf(),
        port,
        jwt_secret: JWT_SECRET.to_string(),
        wal_fsync_policy: "per_op".to_string(),
    });

    supervisor.start(READY_TIMEOUT).await.expect("server start");

    // Seed N unique-tag OR removes. Each remove appends its client tag to the
    // key's tombstone set and increments the gauge by that tag's byte length.
    // Tags are `tomb-000000`..`tomb-0000NN` — a fixed 11-byte width — so the
    // accumulated total is exactly N * 11.
    let mut c = SoakClient::connect(supervisor.addr(), 0, JWT_SECRET)
        .await
        .expect("client connect");
    for i in 0..N_TOMBS {
        let tag = format!("tomb-{i:06}");
        assert_eq!(
            tag.len(),
            11,
            "tag width must stay fixed for exact accounting"
        );
        c.or_add(OR_MAP, OR_KEY, &tag, i as i64, 1, i as u32)
            .await
            .expect("or_add");
        c.or_remove(OR_MAP, OR_KEY, &tag).await.expect("or_remove");
    }
    drop(c);

    let expected: u64 = N_TOMBS as u64 * 11;

    // Pre-kill: the live gauge reflects the accumulated tombstone bytes.
    let pre = scrape_tombstone_bytes(port)
        .await
        .expect("pre-kill /metrics must expose the gauge");
    assert!(
        pre > 0,
        "pre-kill gauge must be non-zero after seeding removes"
    );
    assert_eq!(pre, expected, "pre-kill gauge should equal N * tag_len");

    // kill -9 + relaunch against the same data dir (real WAL recovery).
    supervisor
        .restart(READY_TIMEOUT)
        .await
        .expect("server restart");

    // FIRST post-restart scrape, with NO intervening OR mutation. The value must
    // come from boot reconciliation alone. On a fresh process without the seed
    // this would read 0/absent; a non-zero T here proves the gauge survives.
    let post = scrape_tombstone_bytes(port)
        .await
        .expect("post-restart /metrics must expose the reconciled gauge");
    assert_eq!(
        post, pre,
        "first post-restart scrape must reflect the reconciled total ({pre}), not 0"
    );

    supervisor.shutdown().await;
}
