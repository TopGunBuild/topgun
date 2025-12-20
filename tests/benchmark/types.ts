/**
 * Native Benchmark Harness Types
 *
 * Core interfaces for the TopGun benchmark system.
 */

export type LoadMode = 'throttled' | 'saturate';

export interface BenchmarkConfig {
  /** WebSocket server URL (e.g., ws://localhost:8080) */
  serverUrl: string;
  /** JWT authentication token */
  token: string;
  /** Number of concurrent WebSocket connections */
  connections: number;
  /** Number of operations per batch */
  batchSize: number;
  /** Total test duration in milliseconds */
  durationMs: number;
  /** Interval between batches in milliseconds (per connection) - only used in 'throttled' mode */
  intervalMs: number;
  /** Number of maps to distribute operations across */
  mapCount: number;
  /** Warmup period before recording metrics (ms) */
  warmupMs: number;
  /** Load generation mode: 'throttled' (interval-based) or 'saturate' (continuous loop) */
  mode: LoadMode;
  /** Max pending operations per connection before backpressure (saturate mode) */
  maxPendingOps: number;
}

export interface ThroughputMetrics {
  /** Total operations sent */
  totalOpsSent: number;
  /** Total operations acknowledged by server */
  totalOpsAcked: number;
  /** Average operations per second */
  opsPerSec: number;
  /** Peak operations per second (1-second window) */
  peakOpsPerSec: number;
}

export interface LatencyMetrics {
  /** Minimum latency in ms */
  min: number;
  /** Maximum latency in ms */
  max: number;
  /** Mean latency in ms */
  mean: number;
  /** Median (50th percentile) in ms */
  p50: number;
  /** 95th percentile in ms */
  p95: number;
  /** 99th percentile in ms */
  p99: number;
  /** 99.9th percentile in ms */
  p999: number;
  /** Standard deviation in ms */
  stdDev: number;
}

export interface ReliabilityMetrics {
  /** Percentage of successful operations (0-1) */
  successRate: number;
  /** Percentage of failed operations (0-1) */
  errorRate: number;
  /** Number of connection errors */
  connectionErrors: number;
  /** Number of timeout errors */
  timeoutErrors: number;
  /** Number of protocol errors */
  protocolErrors: number;
}

export interface BenchmarkResult {
  /** Scenario name (smoke, throughput, latency, stress) */
  scenario: string;
  /** Configuration used for this run */
  config: BenchmarkConfig;
  /** ISO timestamp when benchmark started */
  startTime: string;
  /** Actual test duration in seconds */
  durationSec: number;
  /** Throughput statistics */
  throughput: ThroughputMetrics;
  /** Latency statistics */
  latency: LatencyMetrics;
  /** Reliability statistics */
  reliability: ReliabilityMetrics;
  /** Version of TopGun being tested */
  version: string;
  /** Whether benchmark passed all thresholds */
  passed: boolean;
  /** Failure reasons if passed is false */
  failureReasons: string[];
}

export interface ScenarioThresholds {
  /** Minimum ops/sec required */
  minThroughput?: number;
  /** Maximum p99 latency allowed (ms) */
  maxP99Latency?: number;
  /** Maximum error rate allowed (0-1) */
  maxErrorRate?: number;
  /** Maximum connection errors allowed */
  maxConnectionErrors?: number;
}

export type ScenarioName = 'smoke' | 'throughput' | 'latency' | 'stress';

export interface ScenarioConfig {
  name: ScenarioName;
  description: string;
  config: Partial<BenchmarkConfig>;
  thresholds: ScenarioThresholds;
}

/** Exit codes for CI integration */
export const EXIT_CODES = {
  SUCCESS: 0,
  BENCHMARK_FAILED: 1,
  CONNECTION_ERROR: 2,
  THRESHOLD_EXCEEDED: 3,
  INVALID_CONFIG: 4,
} as const;

export type ExitCode = typeof EXIT_CODES[keyof typeof EXIT_CODES];

/** Message types used in benchmark */
export interface PendingOperation {
  id: string;
  sendTime: number;
  batchSize: number;
}

export interface ConnectionState {
  id: string;
  isAuthenticated: boolean;
  isConnected: boolean;
  pendingOps: Map<string, PendingOperation>;
  opCounter: number;
  sentOps: number;
  ackedOps: number;
  errors: number;
}
