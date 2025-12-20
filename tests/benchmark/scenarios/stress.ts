/**
 * Stress Test Scenario
 *
 * High load test to find the breaking point.
 * Uses many connections and aggressive batch rates.
 */

import { BenchmarkHarness } from '../harness';
import { STRESS_SCENARIO, buildConfig } from '../config';
import { printStressResult, saveResultToJson, checkThresholds, printBanner } from '../reporter';
import type { BenchmarkConfig, ExitCode } from '../types';
import { EXIT_CODES } from '../types';

export async function runStress(overrides: Partial<BenchmarkConfig> = {}): Promise<ExitCode> {
  const config = buildConfig(STRESS_SCENARIO, overrides);

  printBanner('stress', config);

  console.log('WARNING: Stress test uses high load. Server may become unresponsive.');
  console.log('');

  const harness = new BenchmarkHarness(config, {
    onProgress: (sent, acked) => {
      const lossRate = sent > 0 ? ((sent - acked) / sent * 100).toFixed(1) : '0.0';
      process.stdout.write(
        `\rSent: ${sent.toLocaleString()}  Acked: ${acked.toLocaleString()}  Loss: ${lossRate}%`
      );
    },
    onError: (err) => {
      // Suppress individual errors during stress test
    },
  });

  try {
    let result = await harness.run('stress');
    console.log(''); // Clear progress line

    // Check thresholds
    result = checkThresholds(result, STRESS_SCENARIO.thresholds);

    // Print and save results
    printStressResult(result);
    saveResultToJson(result);

    return result.passed ? EXIT_CODES.SUCCESS : EXIT_CODES.THRESHOLD_EXCEEDED;
  } catch (error) {
    console.error('\nStress test failed:', error);
    return EXIT_CODES.CONNECTION_ERROR;
  }
}
