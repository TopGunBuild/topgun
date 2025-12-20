/**
 * Benchmark Configuration
 *
 * Predefined configurations for different benchmark scenarios.
 */

import type { BenchmarkConfig, ScenarioConfig } from './types';

/** Default configuration values */
export const DEFAULT_CONFIG: BenchmarkConfig = {
  serverUrl: process.env.BENCHMARK_URL || 'ws://localhost:8080',
  token: process.env.JWT_TOKEN || '',
  connections: 50,
  batchSize: 10,
  durationMs: 60_000,
  intervalMs: 100,
  mapCount: 10,
  warmupMs: 5_000,
  mode: 'throttled',
  maxPendingOps: 100,
};

/** Smoke test - quick validation for CI */
export const SMOKE_SCENARIO: ScenarioConfig = {
  name: 'smoke',
  description: 'Quick validation test for CI (10 seconds)',
  config: {
    connections: 10,
    batchSize: 5,
    durationMs: 10_000,
    intervalMs: 200,
    mapCount: 5,
    warmupMs: 1_000,
  },
  thresholds: {
    minThroughput: 100,
    maxP99Latency: 500,
    maxErrorRate: 0.01,
    maxConnectionErrors: 0,
  },
};

/** Throughput test - maximum sustainable load */
export const THROUGHPUT_SCENARIO: ScenarioConfig = {
  name: 'throughput',
  description: 'Maximum throughput benchmark (60 seconds, saturate mode)',
  config: {
    connections: 100,
    batchSize: 5,
    durationMs: 60_000,
    intervalMs: 1, // Not used in saturate mode
    mapCount: 20,
    warmupMs: 5_000,
    mode: 'saturate',
    maxPendingOps: 5, // Optimal for ~30K ops/sec with good reliability
  },
  thresholds: {
    minThroughput: 15_000,
    maxP99Latency: 100,
    maxErrorRate: 0.01,
  },
};

/** Latency test - low load, precise percentiles */
export const LATENCY_SCENARIO: ScenarioConfig = {
  name: 'latency',
  description: 'Latency-focused benchmark (30 seconds, low load)',
  config: {
    connections: 20,
    batchSize: 1,
    durationMs: 30_000,
    intervalMs: 100,
    mapCount: 5,
    warmupMs: 3_000,
  },
  thresholds: {
    maxP99Latency: 50,
    maxErrorRate: 0.001,
  },
};

/** Stress test - find breaking point */
export const STRESS_SCENARIO: ScenarioConfig = {
  name: 'stress',
  description: 'Stress test to find breaking point (120 seconds)',
  config: {
    connections: 200,
    batchSize: 20,
    durationMs: 120_000,
    intervalMs: 25,
    mapCount: 50,
    warmupMs: 10_000,
  },
  thresholds: {
    // More lenient thresholds for stress test
    maxErrorRate: 0.05,
  },
};

/** All available scenarios */
export const SCENARIOS: Record<string, ScenarioConfig> = {
  smoke: SMOKE_SCENARIO,
  throughput: THROUGHPUT_SCENARIO,
  latency: LATENCY_SCENARIO,
  stress: STRESS_SCENARIO,
};

/**
 * Merge custom config with scenario defaults
 */
export function buildConfig(
  scenario: ScenarioConfig,
  overrides: Partial<BenchmarkConfig> = {}
): BenchmarkConfig {
  return {
    ...DEFAULT_CONFIG,
    ...scenario.config,
    ...overrides,
  };
}

/**
 * Parse CLI arguments into config overrides
 */
export function parseCliArgs(args: string[]): {
  scenario: string;
  overrides: Partial<BenchmarkConfig>;
} {
  let scenario = 'smoke';
  const overrides: Partial<BenchmarkConfig> = {};

  for (const arg of args) {
    if (arg.startsWith('--')) {
      const [key, value] = arg.slice(2).split('=');
      switch (key) {
        case 'connections':
          overrides.connections = parseInt(value, 10);
          break;
        case 'batch-size':
        case 'batchSize':
          overrides.batchSize = parseInt(value, 10);
          break;
        case 'duration':
          overrides.durationMs = parseInt(value, 10);
          break;
        case 'interval':
          overrides.intervalMs = parseInt(value, 10);
          break;
        case 'maps':
        case 'mapCount':
          overrides.mapCount = parseInt(value, 10);
          break;
        case 'warmup':
          overrides.warmupMs = parseInt(value, 10);
          break;
        case 'url':
          overrides.serverUrl = value;
          break;
        case 'token':
          overrides.token = value;
          break;
      }
    } else if (!arg.startsWith('-') && SCENARIOS[arg]) {
      scenario = arg;
    }
  }

  return { scenario, overrides };
}
