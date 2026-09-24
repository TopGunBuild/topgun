//! Micro-bench of `merkle_leaf_hash` on an OR-Map slot.
//!
//! The OR arm runs once per OR write (the Merkle observer recomputes the key's
//! leaf), so its cost scales with the slot. Inputs: N ∈ {1 000, 10 000} records
//! with 29-char distinct tags and no tombstones. Each N is timed over 200 reps
//! after 20 warm-up reps; the median and p99 per N are printed.
//!
//! `cargo bench --bench leaf_hash`

use std::hint::black_box;
use std::time::{Duration, Instant};

use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;
use topgun_server::storage::merkle_leaf_hash;
use topgun_server::storage::record::{OrMapEntry, RecordValue};

const WARMUP_REPS: usize = 20;
const REPS: usize = 200;

fn or_slot(n: usize) -> RecordValue {
    let ts = Timestamp {
        millis: 1_700_000_000_000,
        counter: 0,
        node_id: "node-a".to_string(),
    };
    RecordValue::OrMap {
        records: (0..n)
            .map(|i| OrMapEntry {
                value: Value::Null,
                tag: format!("{i:020}:0:node-a"),
                timestamp: ts.clone(),
            })
            .collect(),
        tombstones: Vec::new(),
    }
}

fn main() {
    for n in [1_000_usize, 10_000] {
        let value = or_slot(n);
        for _ in 0..WARMUP_REPS {
            black_box(merkle_leaf_hash(black_box("k"), black_box(&value)));
        }
        let mut samples: Vec<Duration> = (0..REPS)
            .map(|_| {
                let start = Instant::now();
                black_box(merkle_leaf_hash(black_box("k"), black_box(&value)));
                start.elapsed()
            })
            .collect();
        samples.sort_unstable();
        let median = samples[REPS / 2];
        let p99 = samples[REPS * 99 / 100];
        println!(
            "leaf_hash N={n} reps={REPS} median_us={:.3} p99_us={:.3}",
            median.as_secs_f64() * 1e6,
            p99.as_secs_f64() * 1e6
        );
    }
}
