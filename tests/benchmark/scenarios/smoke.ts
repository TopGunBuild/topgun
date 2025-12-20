/**
 * Smoke Test Scenario
 *
 * Quick validation test for CI - 10 seconds, minimal load.
 * Validates basic connectivity and protocol correctness.
 */

import { BenchmarkHarness } from '../harness';
import { SMOKE_SCENARIO, buildConfig } from '../config';
import { printSmokeResult, saveResultToJson, checkThresholds } from '../reporter';
import type { BenchmarkConfig, BenchmarkResult, ExitCode } from '../types';
import { EXIT_CODES } from '../types';

export async function runSmoke(overrides: Partial<BenchmarkConfig> = {}): Promise<ExitCode> {
  const config = buildConfig(SMOKE_SCENARIO, overrides);

  console.log('');
  console.log('Starting smoke test...');
  console.log(`Server: ${config.serverUrl}`);
  console.log(`Connections: ${config.connections}`);
  console.log(`Duration: ${config.durationMs / 1000} seconds`);
  console.log('');

  const harness = new BenchmarkHarness(config, {
    onError: (err) => {
      console.error('Error:', err.message);
    },
  });

  try {
    let result = await harness.run('smoke');

    // Check thresholds
    result = checkThresholds(result, SMOKE_SCENARIO.thresholds);

    // Print and save results
    printSmokeResult(result);
    saveResultToJson(result);

    return result.passed ? EXIT_CODES.SUCCESS : EXIT_CODES.THRESHOLD_EXCEEDED;
  } catch (error: any) {
    if (error.code === 'ECONNREFUSED') {
      console.error('');
      console.error('ERROR: Cannot connect to server at', config.serverUrl);
      console.error('');
      console.error('Make sure the server is running:');
      console.error('  pnpm start:server');
      console.error('');
    } else {
      console.error('Smoke test failed:', error.message || error);
    }
    return EXIT_CODES.CONNECTION_ERROR;
  }
}
