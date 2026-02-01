/**
 * Baseline benchmark for scoreDocument performance.
 * Capture baseline "before" metrics.
 */
import { SearchCoordinator } from '../SearchCoordinator';

const SIZES = [100, 1000, 10000];

async function runBaseline() {
  console.log('=== scoreDocument Baseline Benchmark ===\n');

  for (const size of SIZES) {
    const coordinator = new SearchCoordinator();
    coordinator.enableSearch('test', { fields: ['title', 'body'] });

    // Build index with N documents
    const entries: Array<[string, Record<string, unknown>]> = [];
    for (let i = 0; i < size; i++) {
      entries.push([`doc-${i}`, {
        title: `Document ${i} about wireless technology`,
        body: `This is the body of document ${i} discussing various topics.`
      }]);
    }
    coordinator.buildIndexFromEntries('test', entries);

    // Create subscription
    const results = coordinator.subscribe(
      'client-1',
      'sub-1',
      'test',
      'wireless technology',
      { limit: 10 }
    );

    // Measure scoreDocument for single document update
    const iterations = 100;
    const start = performance.now();

    for (let i = 0; i < iterations; i++) {
      coordinator.onDataChange('test', 'doc-500', {
        title: 'Updated wireless mouse',
        body: 'New content about wireless devices'
      }, 'update');
    }

    const elapsed = performance.now() - start;
    const avgMs = elapsed / iterations;

    console.log(`${size} docs: ${avgMs.toFixed(3)}ms per update`);

    coordinator.clear();
  }
}

runBaseline();
