/**
 * Benchmark Reporter
 *
 * Console and JSON output for benchmark results.
 */

import * as fs from 'fs';
import * as path from 'path';
import type { BenchmarkResult, ScenarioThresholds } from './types';

const RESULTS_DIR = path.join(__dirname, 'results');

/**
 * Format number with thousands separator
 */
function formatNumber(n: number): string {
  return n.toLocaleString('en-US');
}

/**
 * Format latency value
 */
function formatLatency(ms: number): string {
  if (ms < 1) {
    return `${(ms * 1000).toFixed(0)} us`;
  }
  return `${ms.toFixed(1)} ms`;
}

/**
 * Format percentage
 */
function formatPercent(rate: number): string {
  return `${(rate * 100).toFixed(3)}%`;
}

/**
 * Pad string for alignment
 */
function pad(str: string, width: number): string {
  return str.padStart(width);
}

/**
 * Print a box line
 */
function boxLine(content: string, width = 68): void {
  const padded = content.padEnd(width - 2);
  console.log(`\u2551  ${padded}\u2551`);
}

/**
 * Print box top
 */
function boxTop(width = 68): void {
  console.log('\u2554' + '\u2550'.repeat(width) + '\u2557');
}

/**
 * Print box bottom
 */
function boxBottom(width = 68): void {
  console.log('\u255A' + '\u2550'.repeat(width) + '\u255D');
}

/**
 * Print box separator
 */
function boxSeparator(width = 68): void {
  console.log('\u2560' + '\u2550'.repeat(width) + '\u2563');
}

/**
 * Print header line
 */
function boxHeader(title: string, width = 68): void {
  const padding = Math.floor((width - title.length) / 2);
  const content = ' '.repeat(padding) + title + ' '.repeat(width - padding - title.length);
  console.log(`\u2551${content}\u2551`);
}

/**
 * Print smoke test results
 */
export function printSmokeResult(result: BenchmarkResult): void {
  console.log('');
  boxTop();
  boxHeader('TOPGUN SMOKE TEST');
  boxSeparator();
  boxLine(`Duration:           ${pad(result.durationSec.toFixed(1), 10)} sec`);
  boxLine(`Connections:        ${pad(result.config.connections.toString(), 10)}`);
  boxLine(`Operations:         ${pad(formatNumber(result.throughput.totalOpsAcked), 10)}`);
  boxLine(`Throughput:         ${pad(formatNumber(result.throughput.opsPerSec), 10)} ops/sec`);
  boxLine(`p99 Latency:        ${pad(formatLatency(result.latency.p99), 10)}`);
  boxLine(`Error Rate:         ${pad(formatPercent(result.reliability.errorRate), 10)}`);
  boxSeparator();

  const status = result.passed ? '\u2705 PASSED' : '\u274C FAILED';
  boxLine(`Status: ${status}`);

  if (!result.passed && result.failureReasons.length > 0) {
    boxLine('');
    boxLine('Failure reasons:');
    for (const reason of result.failureReasons) {
      boxLine(`  - ${reason}`);
    }
  }

  boxBottom();
  console.log('');
}

/**
 * Print throughput test results
 */
export function printThroughputResult(result: BenchmarkResult): void {
  console.log('');
  boxTop();
  boxHeader('TOPGUN THROUGHPUT BENCHMARK');
  boxSeparator();
  boxLine('THROUGHPUT');
  boxLine(`  Total Operations:     ${pad(formatNumber(result.throughput.totalOpsAcked), 12)}`);
  boxLine(`  Average:              ${pad(formatNumber(result.throughput.opsPerSec), 12)} ops/sec`);
  boxLine(`  Peak:                 ${pad(formatNumber(result.throughput.peakOpsPerSec), 12)} ops/sec`);
  boxSeparator();
  boxLine('LATENCY');
  boxLine(`  min:                  ${pad(formatLatency(result.latency.min), 12)}`);
  boxLine(`  p50:                  ${pad(formatLatency(result.latency.p50), 12)}`);
  boxLine(`  p95:                  ${pad(formatLatency(result.latency.p95), 12)}`);
  boxLine(`  p99:                  ${pad(formatLatency(result.latency.p99), 12)}`);
  boxLine(`  p99.9:                ${pad(formatLatency(result.latency.p999), 12)}`);
  boxLine(`  max:                  ${pad(formatLatency(result.latency.max), 12)}`);
  boxSeparator();
  boxLine('RELIABILITY');
  boxLine(`  Success Rate:         ${pad(formatPercent(result.reliability.successRate), 12)}`);
  boxLine(`  Connection Errors:    ${pad(result.reliability.connectionErrors.toString(), 12)}`);
  boxLine(`  Timeout Errors:       ${pad(result.reliability.timeoutErrors.toString(), 12)}`);
  boxBottom();
  console.log('');
}

/**
 * Print latency test results
 */
export function printLatencyResult(result: BenchmarkResult): void {
  console.log('');
  boxTop();
  boxHeader('TOPGUN LATENCY BENCHMARK');
  boxSeparator();
  boxLine('LATENCY DISTRIBUTION');
  boxLine(`  min:                  ${pad(formatLatency(result.latency.min), 12)}`);
  boxLine(`  p50 (median):         ${pad(formatLatency(result.latency.p50), 12)}`);
  boxLine(`  p95:                  ${pad(formatLatency(result.latency.p95), 12)}`);
  boxLine(`  p99:                  ${pad(formatLatency(result.latency.p99), 12)}`);
  boxLine(`  p99.9:                ${pad(formatLatency(result.latency.p999), 12)}`);
  boxLine(`  max:                  ${pad(formatLatency(result.latency.max), 12)}`);
  boxLine(`  mean:                 ${pad(formatLatency(result.latency.mean), 12)}`);
  boxLine(`  std dev:              ${pad(formatLatency(result.latency.stdDev), 12)}`);
  boxSeparator();
  boxLine('THROUGHPUT');
  boxLine(`  Operations:           ${pad(formatNumber(result.throughput.totalOpsAcked), 12)}`);
  boxLine(`  Rate:                 ${pad(formatNumber(result.throughput.opsPerSec), 12)} ops/sec`);
  boxSeparator();

  const status = result.passed ? '\u2705 PASSED' : '\u274C FAILED';
  boxLine(`Status: ${status}`);

  boxBottom();
  console.log('');
}

/**
 * Print stress test results
 */
export function printStressResult(result: BenchmarkResult): void {
  console.log('');
  boxTop();
  boxHeader('TOPGUN STRESS TEST');
  boxSeparator();
  boxLine('LOAD');
  boxLine(`  Connections:          ${pad(result.config.connections.toString(), 12)}`);
  boxLine(`  Duration:             ${pad(result.durationSec.toFixed(1) + ' sec', 12)}`);
  boxSeparator();
  boxLine('THROUGHPUT');
  boxLine(`  Total Operations:     ${pad(formatNumber(result.throughput.totalOpsSent), 12)}`);
  boxLine(`  Acknowledged:         ${pad(formatNumber(result.throughput.totalOpsAcked), 12)}`);
  boxLine(`  Average:              ${pad(formatNumber(result.throughput.opsPerSec), 12)} ops/sec`);
  boxLine(`  Peak:                 ${pad(formatNumber(result.throughput.peakOpsPerSec), 12)} ops/sec`);
  boxSeparator();
  boxLine('LATENCY UNDER STRESS');
  boxLine(`  p50:                  ${pad(formatLatency(result.latency.p50), 12)}`);
  boxLine(`  p95:                  ${pad(formatLatency(result.latency.p95), 12)}`);
  boxLine(`  p99:                  ${pad(formatLatency(result.latency.p99), 12)}`);
  boxLine(`  max:                  ${pad(formatLatency(result.latency.max), 12)}`);
  boxSeparator();
  boxLine('ERRORS');
  boxLine(`  Error Rate:           ${pad(formatPercent(result.reliability.errorRate), 12)}`);
  boxLine(`  Connection Errors:    ${pad(result.reliability.connectionErrors.toString(), 12)}`);
  boxLine(`  Timeout Errors:       ${pad(result.reliability.timeoutErrors.toString(), 12)}`);
  boxLine(`  Protocol Errors:      ${pad(result.reliability.protocolErrors.toString(), 12)}`);
  boxBottom();
  console.log('');
}

/**
 * Print result based on scenario type
 */
export function printResult(result: BenchmarkResult): void {
  switch (result.scenario) {
    case 'smoke':
      printSmokeResult(result);
      break;
    case 'throughput':
      printThroughputResult(result);
      break;
    case 'latency':
      printLatencyResult(result);
      break;
    case 'stress':
      printStressResult(result);
      break;
    default:
      printThroughputResult(result);
  }
}

/**
 * Save result to JSON file
 */
export function saveResultToJson(result: BenchmarkResult): string {
  // Ensure results directory exists
  if (!fs.existsSync(RESULTS_DIR)) {
    fs.mkdirSync(RESULTS_DIR, { recursive: true });
  }

  const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
  const filename = `${result.scenario}-${timestamp}.json`;
  const filepath = path.join(RESULTS_DIR, filename);

  fs.writeFileSync(filepath, JSON.stringify(result, null, 2));
  console.log(`Results saved to: ${filepath}`);

  return filepath;
}

/**
 * Check thresholds and update result
 */
export function checkThresholds(
  result: BenchmarkResult,
  thresholds: ScenarioThresholds
): BenchmarkResult {
  const failures: string[] = [];

  if (thresholds.minThroughput !== undefined) {
    if (result.throughput.opsPerSec < thresholds.minThroughput) {
      failures.push(
        `Throughput ${result.throughput.opsPerSec} ops/sec < required ${thresholds.minThroughput} ops/sec`
      );
    }
  }

  if (thresholds.maxP99Latency !== undefined) {
    if (result.latency.p99 > thresholds.maxP99Latency) {
      failures.push(
        `p99 latency ${result.latency.p99.toFixed(1)}ms > max ${thresholds.maxP99Latency}ms`
      );
    }
  }

  if (thresholds.maxErrorRate !== undefined) {
    if (result.reliability.errorRate > thresholds.maxErrorRate) {
      failures.push(
        `Error rate ${formatPercent(result.reliability.errorRate)} > max ${formatPercent(thresholds.maxErrorRate)}`
      );
    }
  }

  if (thresholds.maxConnectionErrors !== undefined) {
    if (result.reliability.connectionErrors > thresholds.maxConnectionErrors) {
      failures.push(
        `Connection errors ${result.reliability.connectionErrors} > max ${thresholds.maxConnectionErrors}`
      );
    }
  }

  return {
    ...result,
    passed: failures.length === 0,
    failureReasons: failures,
  };
}

/**
 * Print startup banner
 */
export function printBanner(scenario: string, config: { connections: number; durationMs: number; serverUrl: string }): void {
  console.log('');
  boxTop();
  boxHeader(`TOPGUN BENCHMARK: ${scenario.toUpperCase()}`);
  boxSeparator();
  boxLine(`Server:       ${config.serverUrl}`);
  boxLine(`Connections:  ${config.connections}`);
  boxLine(`Duration:     ${(config.durationMs / 1000).toFixed(0)} seconds`);
  boxBottom();
  console.log('');
}

/**
 * Create a .gitkeep file in results directory
 */
export function ensureResultsDir(): void {
  if (!fs.existsSync(RESULTS_DIR)) {
    fs.mkdirSync(RESULTS_DIR, { recursive: true });
  }
  const gitkeep = path.join(RESULTS_DIR, '.gitkeep');
  if (!fs.existsSync(gitkeep)) {
    fs.writeFileSync(gitkeep, '');
  }
}
