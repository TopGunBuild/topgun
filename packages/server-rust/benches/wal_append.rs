//! WAL-only micro-harness: concurrent appends through the public `WalWriter`
//! API, spread over the WAL partitions, with append latency recorded per call.
//!
//! Every partition is appended to once, untimed, before the timed window, so
//! the window measures appends to open segments.
//!
//! It touches nothing but `WalWriter::open_over_store` and `Wal::append`, so the
//! same file builds on any commit that has them and a before/after pair
//! measures the WAL alone. The store the WAL is opened over keeps the applied
//! watermarks and is read only at the open and by collection ticks: nothing
//! flushes into it.
//!
//! ```text
//! cargo bench --bench wal_append -- --policy per_op --partitions 271 \
//!     --appenders 8 --duration 10 --repeat 3 --json-output wal_append.json
//! ```

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use hdrhistogram::Histogram;
use serde::Serialize;
use sha2::{Digest, Sha256};
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_server::storage::datastores::RedbDataStore;
use topgun_server::storage::map_data_store::MapDataStore;
use topgun_server::storage::record::RecordValue;
use topgun_server::storage::wal::{
    Wal, WalEntry, WalFsyncPolicy, WalOp, WalStorePayload, WalWriter,
};

/// Bytes of the string value every appended record carries.
const VALUE_BYTES: usize = 100;

#[derive(Debug, Parser)]
#[command(about = "Concurrent WAL appends over the public WalWriter API")]
struct Args {
    /// fsync policy: `per_op`, `batched` or `none`.
    #[arg(long, default_value = "batched")]
    policy: String,
    /// Partitions the appends are spread over, round robin by sequence.
    #[arg(long, default_value_t = 271)]
    partitions: u32,
    /// Concurrent appenders; defaults to the host's available parallelism.
    #[arg(long)]
    appenders: Option<usize>,
    /// Seconds each run appends for.
    #[arg(long, default_value_t = 10)]
    duration: u64,
    /// Runs, each over a fresh WAL directory and store.
    #[arg(long, default_value_t = 1)]
    repeat: usize,
    /// Writes the report as JSON to this file.
    #[arg(long)]
    json_output: Option<PathBuf>,
    /// Passed by `cargo bench` to every bench binary; ignored.
    #[arg(long, hide = true)]
    bench: bool,
}

#[derive(Debug, Serialize)]
struct Build {
    binary_sha256: Option<String>,
    git_commit: Option<String>,
    git_dirty: Option<bool>,
}

#[derive(Debug, Serialize)]
struct Host {
    available_parallelism: usize,
}

#[derive(Debug, Serialize)]
struct Config {
    policy: String,
    partitions: u32,
    appenders: usize,
    duration_s: u64,
    repeat: usize,
    value_bytes: usize,
}

#[derive(Debug, Serialize)]
struct Run {
    appends: u64,
    elapsed_s: f64,
    appends_per_sec: f64,
    p50_us: u64,
    p99_us: u64,
    p999_us: u64,
    max_us: u64,
}

#[derive(Debug, Serialize)]
struct Report {
    build: Build,
    host: Host,
    config: Config,
    runs: Vec<Run>,
}

fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The SHA-256 of the running binary: release builds are not byte
/// reproducible, so the launched binary's hash is the provenance that counts.
fn binary_sha256() -> Option<String> {
    let bytes = std::fs::read(std::env::current_exe().ok()?).ok()?;
    Some(hex::encode(Sha256::digest(bytes)))
}

fn entry(sequence: u64, value: &str) -> WalEntry {
    let timestamp = Timestamp {
        millis: sequence,
        counter: 0,
        node_id: "bench".to_string(),
    };
    WalEntry {
        map: "bench".to_string(),
        key: format!("k{sequence}"),
        op: WalOp::Store {
            value: WalStorePayload::Record(RecordValue::Lww {
                value: Value::String(value.to_string()),
                timestamp: timestamp.clone(),
                ttl_ms: None,
            }),
        },
        timestamp: Some(timestamp),
        sequence,
    }
}

async fn run_once(
    policy: WalFsyncPolicy,
    partitions: u32,
    appenders: usize,
    duration: Duration,
) -> anyhow::Result<Run> {
    let dir = tempfile::tempdir()?;
    let store: Arc<dyn MapDataStore> = Arc::new(RedbDataStore::new(dir.path().join("data"))?);
    let wal = WalWriter::open_over_store(dir.path().join("wal"), policy, Arc::clone(&store))?;
    let value = "v".repeat(VALUE_BYTES);
    // Untimed: the first append to a partition creates its segment file, so
    // without this a short run would time segment creation, not appends.
    for partition in 0..partitions {
        let sequence = u64::from(partition) + 1;
        wal.append(partition, &entry(sequence, &value)).await?;
    }
    let next = Arc::new(AtomicU64::new(u64::from(partitions) + 1));
    let started = Instant::now();
    let deadline = started + duration;
    let mut tasks = Vec::with_capacity(appenders);
    for _ in 0..appenders {
        let wal = Arc::clone(&wal);
        let next = Arc::clone(&next);
        let value = value.clone();
        tasks.push(tokio::spawn(async move {
            let mut histogram = Histogram::<u64>::new(3)?;
            while Instant::now() < deadline {
                let sequence = next.fetch_add(1, Ordering::Relaxed);
                let entry = entry(sequence, &value);
                let partition = u32::try_from(sequence % u64::from(partitions))?;
                let at = Instant::now();
                wal.append(partition, &entry).await?;
                histogram.record(u64::try_from(at.elapsed().as_micros())?.max(1))?;
            }
            anyhow::Ok(histogram)
        }));
    }
    let mut histogram = Histogram::<u64>::new(3)?;
    for task in tasks {
        histogram.add(task.await??)?;
    }
    let elapsed = started.elapsed().as_secs_f64();
    // Stop the writer thread and the collection tick before the directory
    // goes: left running, a segment sealed by age is followed by one created
    // in the removed directory, which is a WAL fail-stop.
    wal.finish(false).await?;
    drop(wal);
    drop(store);
    let appends = histogram.len();
    // Precision loss only past 2^53 appends.
    #[allow(clippy::cast_precision_loss)]
    let appends_per_sec = appends as f64 / elapsed;
    Ok(Run {
        appends,
        elapsed_s: elapsed,
        appends_per_sec,
        p50_us: histogram.value_at_quantile(0.50),
        p99_us: histogram.value_at_quantile(0.99),
        p999_us: histogram.value_at_quantile(0.999),
        max_us: histogram.max(),
    })
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let policy = WalFsyncPolicy::from_str(&args.policy)?;
    let available_parallelism = std::thread::available_parallelism().map_or(1, usize::from);
    let appenders = args.appenders.unwrap_or(available_parallelism).max(1);
    let partitions = args.partitions.max(1);
    let duration = Duration::from_secs(args.duration);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut runs = Vec::with_capacity(args.repeat);
    for run in 1..=args.repeat.max(1) {
        let result = runtime.block_on(run_once(policy, partitions, appenders, duration))?;
        println!(
            "wal_append run={run} policy={} partitions={partitions} appenders={appenders} \
             appends={} appends_per_sec={:.0} p50_us={} p99_us={} p999_us={} max_us={}",
            args.policy,
            result.appends,
            result.appends_per_sec,
            result.p50_us,
            result.p99_us,
            result.p999_us,
            result.max_us
        );
        runs.push(result);
    }

    let report = Report {
        build: Build {
            binary_sha256: binary_sha256(),
            git_commit: command_line("git", &["rev-parse", "HEAD"]),
            git_dirty: command_line("git", &["status", "--porcelain"]).map(|s| !s.is_empty()),
        },
        host: Host {
            available_parallelism,
        },
        config: Config {
            policy: args.policy,
            partitions,
            appenders,
            duration_s: args.duration,
            repeat: runs.len(),
            value_bytes: VALUE_BYTES,
        },
        runs,
    };
    if let Some(path) = &args.json_output {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}
