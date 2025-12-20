/**
 * Flood Test Scenario
 *
 * No backpressure mode - like k6 behavior.
 * Sends operations at fixed interval without waiting for ACKs.
 * Used for comparison with k6 benchmark results.
 */

import { BenchmarkHarness } from '../harness';
import { FLOOD_SCENARIO, buildConfig } from '../config';
import { printThroughputResult, saveResultToJson, checkThresholds, printBanner } from '../reporter';
import type { BenchmarkConfig, ExitCode } from '../types';
import { EXIT_CODES } from '../types';

export async function runFlood(overrides: Partial<BenchmarkConfig> = {}): Promise<ExitCode> {
  const config = buildConfig(FLOOD_SCENARIO, overrides);

  printBanner('flood', config);
  console.log('Mode: FLOOD (no backpressure, like k6)\n');

  const harness = new BenchmarkHarness(config, {
    onProgress: (sent, acked) => {
      process.stdout.write(`\rSent: ${sent.toLocaleString()}  Acked: ${acked.toLocaleString()}`);
    },
    onError: (err) => {
      console.error('\nError:', err.message);
    },
  });

  try {
    let result = await harness.run('flood');
    console.log(''); // Clear progress line

    // Check thresholds
    result = checkThresholds(result, FLOOD_SCENARIO.thresholds);

    // Print and save results
    printThroughputResult(result);
    saveResultToJson(result);

    return result.passed ? EXIT_CODES.SUCCESS : EXIT_CODES.THRESHOLD_EXCEEDED;
  } catch (error) {
    console.error('\nFlood test failed:', error);
    return EXIT_CODES.CONNECTION_ERROR;
  }
}
