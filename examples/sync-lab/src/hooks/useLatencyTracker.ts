import { useState, useCallback, useRef, useEffect } from 'react';
import type { LWWMap } from '@topgunbuild/core';
import type { TopGunClient } from '@topgunbuild/client';

export interface LatencyStats {
  min: number;
  max: number;
  avg: number;
  p50: number;
  p95: number;
  p99: number;
  samples: number[];
}

export interface UseLatencyTrackerReturn {
  /** Last measured read latency in ms */
  lastReadLatency: number;
  /** Number of pending (unacked) operations */
  pendingOps: number;
  /** Run a write benchmark: N sequential writes, return per-write latencies */
  runBenchmark: (map: LWWMap<string, any>, count: number) => LatencyStats;
  /** Calculate stats from a raw sample array */
  calcStats: (samples: number[]) => LatencyStats;
}

function percentile(sorted: number[], p: number): number {
  const idx = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, idx)];
}

export function calcStats(samples: number[]): LatencyStats {
  if (samples.length === 0) {
    return { min: 0, max: 0, avg: 0, p50: 0, p95: 0, p99: 0, samples: [] };
  }
  const sorted = [...samples].sort((a, b) => a - b);
  const sum = sorted.reduce((a, b) => a + b, 0);
  return {
    min: sorted[0],
    max: sorted[sorted.length - 1],
    avg: sum / sorted.length,
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
    samples,
  };
}

/**
 * Hook that tracks read latency and pending operation count.
 * Polls getPendingOpsCount() every 500ms as specified in R5.
 */
export function useLatencyTracker(
  map: LWWMap<string, any> | null,
  client: TopGunClient | null,
): UseLatencyTrackerReturn {
  const [lastReadLatency, setLastReadLatency] = useState(0);
  const [pendingOps, setPendingOps] = useState(0);
  const clientRef = useRef(client);

  useEffect(() => {
    clientRef.current = client;
  }, [client]);

  // Measure read latency periodically
  useEffect(() => {
    if (!map) return;

    const measureRead = () => {
      const start = performance.now();
      // Perform an actual read operation
      map.get('__latency_probe__');
      const elapsed = performance.now() - start;
      setLastReadLatency(elapsed);
    };

    measureRead();
    const interval = setInterval(measureRead, 1000);
    return () => clearInterval(interval);
  }, [map]);

  // Poll pending ops count every 500ms
  useEffect(() => {
    if (!client) return;

    const poll = () => {
      const c = clientRef.current;
      if (c) {
        try {
          setPendingOps(c.getPendingOpsCount());
        } catch {
          // Client may be closed
          setPendingOps(0);
        }
      }
    };

    poll();
    const interval = setInterval(poll, 500);
    return () => clearInterval(interval);
  }, [client]);

  const runBenchmark = useCallback(
    (benchMap: LWWMap<string, any>, count: number): LatencyStats => {
      const samples: number[] = [];
      for (let i = 0; i < count; i++) {
        const key = `bench:${Date.now()}:${i}`;
        const start = performance.now();
        benchMap.set(key, `value-${i}`);
        const elapsed = performance.now() - start;
        samples.push(elapsed);
      }
      return calcStats(samples);
    },
    [],
  );

  return {
    lastReadLatency,
    pendingOps,
    runBenchmark,
    calcStats,
  };
}
