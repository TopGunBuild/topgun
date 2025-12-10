/**
 * Load Test Utilities
 * Measurement and formatting helpers for performance tests
 */

export interface TimingResult<T> {
  result: T;
  timeMs: number;
}

export interface ThroughputResult {
  ops: number;
  timeMs: number;
  opsPerSec: number;
}

export interface LoadTestResults {
  testName: string;
  metrics: Record<string, number | string>;
}

/**
 * Measures execution time of an async function
 */
export async function measureTime<T>(fn: () => Promise<T>): Promise<TimingResult<T>> {
  const start = performance.now();
  const result = await fn();
  const timeMs = performance.now() - start;
  return { result, timeMs };
}

/**
 * Measures execution time of a sync function
 */
export function measureTimeSync<T>(fn: () => T): TimingResult<T> {
  const start = performance.now();
  const result = fn();
  const timeMs = performance.now() - start;
  return { result, timeMs };
}

/**
 * Calculates throughput (operations per second)
 */
export function calculateThroughput(ops: number, timeMs: number): ThroughputResult {
  const opsPerSec = timeMs > 0 ? (ops / timeMs) * 1000 : 0;
  return { ops, timeMs, opsPerSec };
}

/**
 * Calculates percentile from sorted array
 */
export function percentile(sortedValues: number[], p: number): number {
  if (sortedValues.length === 0) return 0;
  const index = Math.ceil((p / 100) * sortedValues.length) - 1;
  return sortedValues[Math.max(0, index)];
}

/**
 * Calculates statistics from an array of values
 */
export function calculateStats(values: number[]): {
  min: number;
  max: number;
  avg: number;
  median: number;
  p95: number;
  p99: number;
} {
  if (values.length === 0) {
    return { min: 0, max: 0, avg: 0, median: 0, p95: 0, p99: 0 };
  }

  const sorted = [...values].sort((a, b) => a - b);
  const sum = values.reduce((acc, v) => acc + v, 0);

  return {
    min: sorted[0],
    max: sorted[sorted.length - 1],
    avg: sum / values.length,
    median: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
  };
}

/**
 * Formats a number with fixed decimal places
 */
function formatNumber(value: number, decimals = 2): string {
  return value.toFixed(decimals);
}

/**
 * Formats milliseconds to human-readable string
 */
export function formatMs(ms: number): string {
  if (ms < 1) {
    return `${formatNumber(ms * 1000, 0)}μs`;
  }
  if (ms < 1000) {
    return `${formatNumber(ms)}ms`;
  }
  return `${formatNumber(ms / 1000)}s`;
}

/**
 * Creates a bordered box for console output
 */
function createBox(lines: string[], width = 45): string {
  const horizontalLine = '─'.repeat(width - 2);
  const output: string[] = [];

  output.push(`┌${horizontalLine}┐`);

  for (const line of lines) {
    const padding = width - 4 - line.length;
    const leftPad = Math.max(0, padding);
    output.push(`│ ${line}${' '.repeat(leftPad)} │`);
  }

  output.push(`└${horizontalLine}┘`);

  return output.join('\n');
}

/**
 * Formats load test results in a readable box format
 */
export function formatResults(results: LoadTestResults): string {
  const lines: string[] = [];

  // Add test name as header
  lines.push(results.testName);
  lines.push('─'.repeat(results.testName.length));

  // Add metrics
  for (const [key, value] of Object.entries(results.metrics)) {
    const formattedValue = typeof value === 'number'
      ? (key.toLowerCase().includes('time') || key.toLowerCase().includes('latency') || key.toLowerCase().includes('ms'))
        ? formatMs(value)
        : formatNumber(value)
      : value;
    lines.push(`${key}: ${formattedValue}`);
  }

  return createBox(lines);
}

/**
 * Formats multiple test results
 */
export function formatMultipleResults(results: LoadTestResults[]): string {
  return results.map(formatResults).join('\n\n');
}

/**
 * Logs results to console with special formatting
 */
export function logResults(results: LoadTestResults): void {
  console.log('\n' + formatResults(results));
}

/**
 * Creates a progress reporter for long-running operations
 */
export function createProgressReporter(total: number, label = 'Progress') {
  let current = 0;
  const startTime = performance.now();

  return {
    increment(amount = 1): void {
      current += amount;
    },

    get progress(): number {
      return current;
    },

    get percentage(): number {
      return (current / total) * 100;
    },

    get elapsedMs(): number {
      return performance.now() - startTime;
    },

    report(): string {
      const pct = this.percentage.toFixed(1);
      const elapsed = formatMs(this.elapsedMs);
      return `${label}: ${current}/${total} (${pct}%) - ${elapsed}`;
    },
  };
}

/**
 * Runs a function multiple times and collects timing statistics
 */
export async function benchmark<T>(
  fn: () => Promise<T>,
  iterations: number
): Promise<{
  results: T[];
  times: number[];
  stats: ReturnType<typeof calculateStats>;
  throughput: ThroughputResult;
}> {
  const results: T[] = [];
  const times: number[] = [];

  for (let i = 0; i < iterations; i++) {
    const { result, timeMs } = await measureTime(fn);
    results.push(result);
    times.push(timeMs);
  }

  const totalTime = times.reduce((acc, t) => acc + t, 0);

  return {
    results,
    times,
    stats: calculateStats(times),
    throughput: calculateThroughput(iterations, totalTime),
  };
}

/**
 * Generates a string of specified size in bytes
 */
export function generatePayload(sizeBytes: number): string {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  let result = '';
  for (let i = 0; i < sizeBytes; i++) {
    result += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return result;
}

/**
 * Generates a test object of approximate size
 */
export function generateLargeObject(sizeBytes: number): Record<string, any> {
  const overhead = 50; // Approximate JSON overhead
  const payloadSize = Math.max(0, sizeBytes - overhead);
  return {
    id: `obj-${Date.now()}`,
    data: generatePayload(payloadSize),
    timestamp: Date.now(),
  };
}
