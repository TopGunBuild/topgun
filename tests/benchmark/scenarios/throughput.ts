/**
 * Throughput Test Scenario
 *
 * Maximum sustainable load test - 60 seconds, high connection count.
 * Measures peak throughput and latency under load.
 */

import { BenchmarkHarness } from '../harness';
import { THROUGHPUT_SCENARIO, buildConfig } from '../config';
import { printThroughputResult, saveResultToJson, checkThresholds, printBanner } from '../reporter';
import type { BenchmarkConfig, ExitCode } from '../types';
import { EXIT_CODES } from '../types';

export async function runThroughput(overrides: Partial<BenchmarkConfig> = {}): Promise<ExitCode> {
  const config = buildConfig(THROUGHPUT_SCENARIO, overrides);

  printBanner('throughput', config);

  const harness = new BenchmarkHarness(config, {
    onProgress: (sent, acked) => {
      process.stdout.write(`\rSent: ${sent.toLocaleString()}  Acked: ${acked.toLocaleString()}`);
    },
    onError: (err) => {
      console.error('\nError:', err.message);
    },
  });

  try {
    let result = await harness.run('throughput');
    console.log(''); // Clear progress line

    // Check thresholds
    result = checkThresholds(result, THROUGHPUT_SCENARIO.thresholds);

    // Print and save results
    printThroughputResult(result);
    saveResultToJson(result);

    return result.passed ? EXIT_CODES.SUCCESS : EXIT_CODES.THRESHOLD_EXCEEDED;
  } catch (error) {
    console.error('\nThroughput test failed:', error);
    return EXIT_CODES.CONNECTION_ERROR;
  }
}
