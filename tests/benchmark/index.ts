#!/usr/bin/env ts-node
/**
 * TopGun Native Benchmark CLI
 *
 * Usage:
 *   pnpm benchmark smoke              # Quick CI validation
 *   pnpm benchmark throughput         # Max throughput test
 *   pnpm benchmark latency            # Latency-focused test
 *   pnpm benchmark stress             # Find breaking point
 *
 *   pnpm benchmark --connections=200  # Custom config
 *
 * Environment variables:
 *   BENCHMARK_URL  - WebSocket server URL (default: ws://localhost:8080)
 *   JWT_TOKEN      - Authentication token
 */

import { parseCliArgs, SCENARIOS } from './config';
import { ensureResultsDir } from './reporter';
import { runSmoke, runThroughput, runLatency, runStress } from './scenarios';
import { EXIT_CODES } from './types';

async function main(): Promise<void> {
  const args = process.argv.slice(2);

  // Show help
  if (args.includes('--help') || args.includes('-h')) {
    printHelp();
    process.exit(0);
  }

  // Parse arguments
  const { scenario, overrides } = parseCliArgs(args);

  // Validate token
  if (!overrides.token && !process.env.JWT_TOKEN) {
    console.error('Error: JWT_TOKEN environment variable or --token argument required');
    console.error('');
    console.error('Generate a token with:');
    console.error('  export JWT_TOKEN=$(node scripts/generate-k6-token.js)');
    console.error('');
    process.exit(EXIT_CODES.INVALID_CONFIG);
  }

  // Ensure results directory exists
  ensureResultsDir();

  // Run the selected scenario
  let exitCode: number;

  switch (scenario) {
    case 'smoke':
      exitCode = await runSmoke(overrides);
      break;
    case 'throughput':
      exitCode = await runThroughput(overrides);
      break;
    case 'latency':
      exitCode = await runLatency(overrides);
      break;
    case 'stress':
      exitCode = await runStress(overrides);
      break;
    default:
      console.error(`Unknown scenario: ${scenario}`);
      console.error(`Available: ${Object.keys(SCENARIOS).join(', ')}`);
      exitCode = EXIT_CODES.INVALID_CONFIG;
  }

  process.exit(exitCode);
}

function printHelp(): void {
  console.log(`
TopGun Native Benchmark

USAGE:
  pnpm benchmark <scenario> [options]

SCENARIOS:
  smoke       Quick CI validation (10 sec, 10 connections)
  throughput  Maximum throughput test (60 sec, 100 connections)
  latency     Latency-focused test (30 sec, 20 connections)
  stress      Find breaking point (120 sec, 200 connections)

OPTIONS:
  --connections=N   Number of WebSocket connections
  --batch-size=N    Operations per batch
  --duration=N      Test duration in milliseconds
  --interval=N      Interval between batches (ms)
  --maps=N          Number of maps to use
  --warmup=N        Warmup period (ms)
  --url=URL         WebSocket server URL
  --token=TOKEN     JWT authentication token

ENVIRONMENT:
  BENCHMARK_URL     WebSocket server URL (default: ws://localhost:8080)
  JWT_TOKEN         Authentication token

EXAMPLES:
  # Run smoke test
  pnpm benchmark smoke

  # Run throughput with custom connections
  pnpm benchmark throughput --connections=200

  # Run with explicit server URL
  pnpm benchmark smoke --url=ws://prod-server:8080

EXIT CODES:
  0  Success
  1  Benchmark failed
  2  Connection error
  3  Threshold exceeded
  4  Invalid configuration
`);
}

// Run
main().catch((err) => {
  console.error('Fatal error:', err);
  process.exit(EXIT_CODES.BENCHMARK_FAILED);
});
