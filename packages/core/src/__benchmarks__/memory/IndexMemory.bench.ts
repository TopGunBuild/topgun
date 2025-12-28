/**
 * Memory Overhead Measurement for Indexes
 *
 * Measures memory consumption of indexed vs non-indexed maps.
 *
 * Run with: node --expose-gc node_modules/.bin/vitest bench -- IndexMemory
 */

import { IndexedLWWMap } from '../../IndexedLWWMap';
import { LWWMap } from '../../LWWMap';
import { HLC } from '../../HLC';
import { simpleAttribute } from '../../query/Attribute';

function measureHeapUsed(): number {
  if (global.gc) {
    global.gc();
    global.gc(); // Run twice for better accuracy
  }
  return process.memoryUsage().heapUsed;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(2)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

function formatPercentage(value: number): string {
  return `${value >= 0 ? '+' : ''}${value.toFixed(1)}%`;
}

async function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

interface MemoryMeasurement {
  baseMemory: number;
  indexedMemory: number;
  overhead: number;
  overheadPercent: number;
  perRecordOverhead: number;
}

async function measureIndexOverhead(size: number): Promise<MemoryMeasurement> {
  const hlc = new HLC('test');

  // Measure base LWWMap
  const before1 = measureHeapUsed();
  const baseMap = new LWWMap<string, any>(hlc);

  for (let i = 0; i < size; i++) {
    baseMap.set(`${i}`, {
      id: i,
      email: `user${i}@test.com`,
      age: i % 100,
      score: Math.random() * 100,
    });
  }

  const baseMemory = measureHeapUsed() - before1;

  // Clear and wait for GC
  baseMap.clear();
  if (global.gc) {
    global.gc();
    global.gc();
  }
  await sleep(100);

  // Measure IndexedLWWMap
  const before2 = measureHeapUsed();
  const indexedMap = new IndexedLWWMap<string, any>(hlc);

  indexedMap.addHashIndex(
    simpleAttribute('email', (r: any) => r.email)
  );
  indexedMap.addHashIndex(
    simpleAttribute('id', (r: any) => r.id)
  );
  indexedMap.addNavigableIndex(
    simpleAttribute('age', (r: any) => r.age)
  );
  indexedMap.addNavigableIndex(
    simpleAttribute('score', (r: any) => r.score)
  );

  for (let i = 0; i < size; i++) {
    indexedMap.set(`${i}`, {
      id: i,
      email: `user${i}@test.com`,
      age: i % 100,
      score: Math.random() * 100,
    });
  }

  const indexedMemory = measureHeapUsed() - before2;

  const overhead = indexedMemory - baseMemory;
  const overheadPercent = (overhead / baseMemory) * 100;
  const perRecordOverhead = overhead / size;

  // Cleanup
  indexedMap.clear();

  return {
    baseMemory,
    indexedMemory,
    overhead,
    overheadPercent,
    perRecordOverhead,
  };
}

async function measureIndexTypeOverhead(
  size: number,
  indexType: 'hash' | 'navigable' | 'both'
): Promise<MemoryMeasurement> {
  const hlc = new HLC('test');

  // Measure base
  const before1 = measureHeapUsed();
  const baseMap = new LWWMap<string, any>(hlc);

  for (let i = 0; i < size; i++) {
    baseMap.set(`${i}`, { id: i, value: i });
  }

  const baseMemory = measureHeapUsed() - before1;

  baseMap.clear();
  if (global.gc) {
    global.gc();
    global.gc();
  }
  await sleep(100);

  // Measure indexed
  const before2 = measureHeapUsed();
  const indexedMap = new IndexedLWWMap<string, any>(hlc);

  if (indexType === 'hash' || indexType === 'both') {
    indexedMap.addHashIndex(
      simpleAttribute('id', (r: any) => r.id)
    );
  }

  if (indexType === 'navigable' || indexType === 'both') {
    indexedMap.addNavigableIndex(
      simpleAttribute('value', (r: any) => r.value)
    );
  }

  for (let i = 0; i < size; i++) {
    indexedMap.set(`${i}`, { id: i, value: i });
  }

  const indexedMemory = measureHeapUsed() - before2;

  const overhead = indexedMemory - baseMemory;
  const overheadPercent = (overhead / baseMemory) * 100;
  const perRecordOverhead = overhead / size;

  indexedMap.clear();

  return {
    baseMemory,
    indexedMemory,
    overhead,
    overheadPercent,
    perRecordOverhead,
  };
}

// Run measurements
async function main() {
  console.log('======================================');
  console.log('Memory Overhead Analysis');
  console.log('======================================\n');

  if (!global.gc) {
    console.warn('⚠️  Warning: --expose-gc flag not detected.');
    console.warn('   Run with: node --expose-gc node_modules/.bin/vitest bench\n');
  }

  console.log('Test 1: Full Index Suite (2 Hash + 2 Navigable indexes)\n');
  console.log('──────────────────────────────────────');

  const sizes = [10_000, 100_000, 1_000_000];

  for (const size of sizes) {
    const result = await measureIndexOverhead(size);
    console.log(`\n${size.toLocaleString()} records:`);
    console.log(`  Base memory:        ${formatBytes(result.baseMemory)}`);
    console.log(`  Indexed memory:     ${formatBytes(result.indexedMemory)}`);
    console.log(`  Overhead:           ${formatBytes(result.overhead)} (${formatPercentage(result.overheadPercent)})`);
    console.log(`  Per-record overhead: ${formatBytes(result.perRecordOverhead)}`);
  }

  console.log('\n\n');
  console.log('Test 2: Index Type Comparison (100,000 records)\n');
  console.log('──────────────────────────────────────');

  const testSize = 100_000;

  const hashResult = await measureIndexTypeOverhead(testSize, 'hash');
  console.log(`\nHash Index Only:`);
  console.log(`  Base memory:    ${formatBytes(hashResult.baseMemory)}`);
  console.log(`  Indexed memory: ${formatBytes(hashResult.indexedMemory)}`);
  console.log(`  Overhead:       ${formatBytes(hashResult.overhead)} (${formatPercentage(hashResult.overheadPercent)})`);

  const navResult = await measureIndexTypeOverhead(testSize, 'navigable');
  console.log(`\nNavigable Index Only:`);
  console.log(`  Base memory:    ${formatBytes(navResult.baseMemory)}`);
  console.log(`  Indexed memory: ${formatBytes(navResult.indexedMemory)}`);
  console.log(`  Overhead:       ${formatBytes(navResult.overhead)} (${formatPercentage(navResult.overheadPercent)})`);

  const bothResult = await measureIndexTypeOverhead(testSize, 'both');
  console.log(`\nBoth Indexes (Hash + Navigable):`);
  console.log(`  Base memory:    ${formatBytes(bothResult.baseMemory)}`);
  console.log(`  Indexed memory: ${formatBytes(bothResult.indexedMemory)}`);
  console.log(`  Overhead:       ${formatBytes(bothResult.overhead)} (${formatPercentage(bothResult.overheadPercent)})`);

  console.log('\n\n');
  console.log('Test 3: Data Size Impact\n');
  console.log('──────────────────────────────────────');

  // Small records
  const hlc = new HLC('test');
  const before1 = measureHeapUsed();
  const smallRecordMap = new IndexedLWWMap<string, { id: number }>(hlc);
  smallRecordMap.addHashIndex(
    simpleAttribute('id', (r: any) => r.id)
  );

  for (let i = 0; i < 100_000; i++) {
    smallRecordMap.set(`${i}`, { id: i });
  }

  const smallRecordMemory = measureHeapUsed() - before1;
  smallRecordMap.clear();

  if (global.gc) {
    global.gc();
    global.gc();
  }
  await sleep(100);

  // Large records
  const before2 = measureHeapUsed();
  const largeRecordMap = new IndexedLWWMap<string, any>(hlc);
  largeRecordMap.addHashIndex(
    simpleAttribute('id', (r: any) => r.id)
  );

  for (let i = 0; i < 100_000; i++) {
    largeRecordMap.set(`${i}`, {
      id: i,
      name: `User ${i}`,
      email: `user${i}@example.com`,
      description: 'Lorem ipsum dolor sit amet, consectetur adipiscing elit.',
      metadata: {
        created: Date.now(),
        updated: Date.now(),
        tags: ['tag1', 'tag2', 'tag3'],
      },
    });
  }

  const largeRecordMemory = measureHeapUsed() - before2;

  console.log(`\nSmall records (100K, ~8 bytes each):`);
  console.log(`  Total memory: ${formatBytes(smallRecordMemory)}`);

  console.log(`\nLarge records (100K, ~200 bytes each):`);
  console.log(`  Total memory: ${formatBytes(largeRecordMemory)}`);

  console.log(`\nRatio: ${(largeRecordMemory / smallRecordMemory).toFixed(2)}×`);

  console.log('\n======================================');
  console.log('Analysis complete');
  console.log('======================================\n');
}

// Run if executed directly
if (require.main === module) {
  main().catch(console.error);
}

export { measureIndexOverhead, measureIndexTypeOverhead };
