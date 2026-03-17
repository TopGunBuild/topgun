use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use hdrhistogram::Histogram;
use parking_lot::Mutex;

use crate::traits::{LatencyStats, MetricsCollector, MetricsSnapshot};

/// HDR histogram-based metrics collector that is safe to share across threads.
///
/// Latencies are stored in microseconds. Each distinct operation name gets its
/// own histogram so percentiles are independent across operation types.
pub struct HdrMetricsCollector {
    /// One histogram per operation name, guarded by a per-entry mutex so that
    /// concurrent writers on different operations never block each other.
    histograms: DashMap<String, Mutex<Histogram<u64>>>,
    /// Simple counters that can be incremented without histogram overhead.
    counters: DashMap<String, Arc<AtomicU64>>,
}

impl HdrMetricsCollector {
    /// Creates a new collector ready to record latencies and counters.
    pub fn new() -> Self {
        Self {
            histograms: DashMap::new(),
            counters: DashMap::new(),
        }
    }

    /// Formats and prints an ASCII report of all recorded operations.
    ///
    /// Columns: operation | count | p50 | p95 | p99 | p99.9 | max  (all in µs)
    pub fn print_report(&self) {
        println!(
            "\n{:<30} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "operation", "count", "p50 µs", "p95 µs", "p99 µs", "p99.9 µs", "max µs"
        );
        println!("{}", "-".repeat(92));

        let mut entries: Vec<(String, LatencyStats)> = self
            .histograms
            .iter()
            .map(|entry| {
                let name = entry.key().clone();
                let stats = histogram_to_stats(&entry.value().lock());
                (name, stats)
            })
            .collect();

        // Sort by operation name for stable output.
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, stats) in entries {
            println!(
                "{:<30} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
                name, stats.count, stats.p50, stats.p95, stats.p99, stats.p999, stats.max
            );
        }

        if !self.counters.is_empty() {
            println!("\nCounters:");
            let mut counters: Vec<(String, u64)> = self
                .counters
                .iter()
                .map(|e| (e.key().clone(), e.value().load(Ordering::Relaxed)))
                .collect();
            counters.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, value) in counters {
                println!("  {}: {}", name, value);
            }
        }
    }
}

impl Default for HdrMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector for HdrMetricsCollector {
    fn record_latency(&self, operation: &str, duration_us: u64) {
        // 3 significant digits gives ~0.1% relative error — standard for latency histograms.
        // 60_000_000 µs = 60 seconds, sufficient for any realistic operation timeout.
        let entry = self.histograms.entry(operation.to_string()).or_insert_with(|| {
            Mutex::new(
                Histogram::<u64>::new_with_max(60_000_000, 3)
                    .expect("valid histogram configuration"),
            )
        });
        // Saturating record so a single out-of-range value never panics the thread.
        entry.lock().saturating_record(duration_us);
    }

    fn increment_counter(&self, name: &str, count: u64) {
        let entry = self
            .counters
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(count, Ordering::Relaxed);
    }

    fn snapshot(&self) -> MetricsSnapshot {
        let latencies: HashMap<String, LatencyStats> = self
            .histograms
            .iter()
            .map(|entry| {
                let stats = histogram_to_stats(&entry.value().lock());
                (entry.key().clone(), stats)
            })
            .collect();

        let counters: HashMap<String, u64> = self
            .counters
            .iter()
            .map(|e| (e.key().clone(), e.value().load(Ordering::Relaxed)))
            .collect();

        MetricsSnapshot { latencies, counters }
    }
}

/// Extracts percentile statistics from a locked histogram.
fn histogram_to_stats(hist: &Histogram<u64>) -> LatencyStats {
    LatencyStats {
        p50: hist.value_at_quantile(0.50),
        p95: hist.value_at_quantile(0.95),
        p99: hist.value_at_quantile(0.99),
        p999: hist.value_at_quantile(0.999),
        min: hist.min(),
        max: hist.max(),
        mean: hist.mean(),
        count: hist.len(),
    }
}
