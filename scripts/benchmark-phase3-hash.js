#!/usr/bin/env node
/**
 * Phase 3.06: Hash Function Benchmark
 *
 * Compares native xxHash64 vs SHA-256 vs FNV-1a
 */

const { createHash } = require('crypto');
const { performance } = require('perf_hooks');
const path = require('path');

// Try to load native module
let nativeHash = null;
let isNativeAvailable = false;

try {
  // Try workspace path first
  const nativePath = path.join(__dirname, '..', 'packages', 'native', 'dist', 'index.js');
  nativeHash = require(nativePath);
  isNativeAvailable = nativeHash.isNativeHashAvailable();
} catch (e) {
  try {
    // Fallback to package name
    nativeHash = require('@topgunbuild/native');
    isNativeAvailable = nativeHash.isNativeHashAvailable();
  } catch (e2) {
    console.log('Native module not available, testing JS fallback only');
  }
}

// FNV-1a hash (JS fallback)
function fnv1aHash(str) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

const testSizes = [32, 64, 128, 256, 512, 1024, 4096];
const iterations = 100_000;

console.log('=== Phase 3.06: Hash Function Benchmark ===\n');
console.log(`Native xxHash64 available: ${isNativeAvailable}`);
console.log(`Iterations per test: ${iterations.toLocaleString()}`);
console.log('');

const results = [];

for (const size of testSizes) {
  const data = Buffer.alloc(size);
  // Fill with some data
  for (let i = 0; i < size; i++) {
    data[i] = i % 256;
  }
  const strData = data.toString('hex');

  console.log(`--- ${size} bytes ---`);

  // Native xxhash64 (if available)
  let nativeOpsPerSec = 0;
  if (isNativeAvailable && nativeHash) {
    const nativeStart = performance.now();
    for (let i = 0; i < iterations; i++) {
      nativeHash.xxhash64AsNumber(data);
    }
    const nativeTime = performance.now() - nativeStart;
    nativeOpsPerSec = Math.round(iterations / nativeTime * 1000);
    console.log(`  xxHash64 (native): ${nativeOpsPerSec.toLocaleString()} ops/sec`);
  }

  // FNV-1a (JS)
  const fnvStart = performance.now();
  for (let i = 0; i < iterations; i++) {
    fnv1aHash(strData);
  }
  const fnvTime = performance.now() - fnvStart;
  const fnvOpsPerSec = Math.round(iterations / fnvTime * 1000);
  console.log(`  FNV-1a (JS):       ${fnvOpsPerSec.toLocaleString()} ops/sec`);

  // Crypto SHA-256
  const cryptoStart = performance.now();
  for (let i = 0; i < iterations; i++) {
    createHash('sha256').update(data).digest();
  }
  const cryptoTime = performance.now() - cryptoStart;
  const cryptoOpsPerSec = Math.round(iterations / cryptoTime * 1000);
  console.log(`  SHA-256 (crypto):  ${cryptoOpsPerSec.toLocaleString()} ops/sec`);

  // Speedups
  if (isNativeAvailable) {
    const speedupVsSha = (cryptoOpsPerSec / nativeOpsPerSec).toFixed(1);
    const speedupVsFnv = (nativeOpsPerSec / fnvOpsPerSec).toFixed(1);
    console.log(`  xxHash64 vs SHA-256: ${(nativeOpsPerSec / cryptoOpsPerSec).toFixed(1)}x faster`);
    console.log(`  xxHash64 vs FNV-1a:  ${speedupVsFnv}x faster`);
  }
  console.log('');

  results.push({
    size,
    nativeOpsPerSec,
    fnvOpsPerSec,
    cryptoOpsPerSec,
  });
}

// Batch benchmark (if native available)
if (isNativeAvailable && nativeHash) {
  console.log('--- Batch Operations (100 items x 64 bytes) ---');
  const batchData = Array(100).fill(null).map(() => Buffer.alloc(64));
  const batchIterations = 10_000;

  const batchStart = performance.now();
  for (let i = 0; i < batchIterations; i++) {
    nativeHash.xxhash64BatchAsNumbers(batchData);
  }
  const batchTime = performance.now() - batchStart;
  const batchHashesPerSec = Math.round((batchIterations * 100) / batchTime * 1000);
  console.log(`  xxHash64Batch: ${batchHashesPerSec.toLocaleString()} hashes/sec`);
  console.log('');
}

// Summary
console.log('=== Summary ===');
console.log('');
console.log('| Size | xxHash64 | FNV-1a | SHA-256 | xxHash64 vs SHA-256 |');
console.log('|------|----------|--------|---------|---------------------|');
for (const r of results) {
  const speedup = r.nativeOpsPerSec > 0 ? (r.nativeOpsPerSec / r.cryptoOpsPerSec).toFixed(1) + 'x' : 'N/A';
  console.log(`| ${r.size} B | ${(r.nativeOpsPerSec / 1e6).toFixed(1)}M | ${(r.fnvOpsPerSec / 1e6).toFixed(1)}M | ${(r.cryptoOpsPerSec / 1e6).toFixed(1)}M | ${speedup} |`);
}

// Export results as JSON
const output = {
  timestamp: new Date().toISOString(),
  nativeAvailable: isNativeAvailable,
  iterations,
  results,
};

console.log('');
console.log('JSON Results:');
console.log(JSON.stringify(output, null, 2));
