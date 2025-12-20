/**
 * Latency Test Scenario
 *
 * Low load test for precise latency measurements.
 * Focuses on percentile accuracy with minimal interference.
 */

import { BenchmarkHarness } from '../harness';
import { LATENCY_SCENARIO, buildConfig } from '../config';
import { printLatencyResult, saveResultToJson, checkThresholds, printBanner } from '../reporter';
import type { BenchmarkConfig, ExitCode } from '../types';
import { EXIT_CODES } from '../types';

export async function runLatency(overrides: Partial<BenchmarkConfig> = {}): Promise<ExitCode> {
  const config = buildConfig(LATENCY_SCENARIO, overrides);

  printBanner('latency', config);

  const harness = new BenchmarkHarness(config, {
    onError: (err) => {
      console.error('Error:', err.message);
    },
  });

  try {
    let result = await harness.run('latency');

    // Check thresholds
    result = checkThresholds(result, LATENCY_SCENARIO.thresholds);

    // Print and save results
    printLatencyResult(result);
    saveResultToJson(result);

    return result.passed ? EXIT_CODES.SUCCESS : EXIT_CODES.THRESHOLD_EXCEEDED;
  } catch (error) {
    console.error('Latency test failed:', error);
    return EXIT_CODES.CONNECTION_ERROR;
  }
}
