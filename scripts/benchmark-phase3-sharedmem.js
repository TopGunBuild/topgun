#!/usr/bin/env node
/**
 * Phase 3.06: SharedArrayBuffer vs postMessage Benchmark
 *
 * Compares zero-copy SharedArrayBuffer transfer vs structured clone
 */

const { Worker, isMainThread, parentPort, workerData } = require('worker_threads');
const { performance } = require('perf_hooks');
const path = require('path');

// Worker code
if (!isMainThread) {
  const { buffer, slotSize, metadataSize, useShared } = workerData;

  if (useShared && buffer) {
    // SharedArrayBuffer mode - read from shared memory
    parentPort.on('message', (msg) => {
      if (msg.type === 'TASK') {
        const { slotIndex, dataLength } = msg;
        // Read from shared memory (zero-copy)
        const dataOffset = slotIndex * slotSize + metadataSize;
        const view = new Uint8Array(buffer, dataOffset, dataLength);

        // Simulate some processing (sum bytes)
        let sum = 0;
        for (let i = 0; i < view.length; i++) {
          sum += view[i];
        }

        parentPort.postMessage({ type: 'DONE', sum });
      }
    });
  } else {
    // postMessage mode - data comes with message
    parentPort.on('message', (msg) => {
      if (msg.type === 'TASK') {
        const data = msg.data;

        // Simulate some processing (sum bytes)
        let sum = 0;
        for (let i = 0; i < data.length; i++) {
          sum += data[i];
        }

        parentPort.postMessage({ type: 'DONE', sum });
      }
    });
  }

  parentPort.postMessage({ type: 'READY' });
} else {
  // Main thread
  async function runBenchmark() {
    console.log('=== Phase 3.06: SharedArrayBuffer vs postMessage Benchmark ===\n');

    // Check SharedArrayBuffer availability
    let sabAvailable = false;
    try {
      new SharedArrayBuffer(1);
      sabAvailable = true;
    } catch (e) {
      console.log('SharedArrayBuffer not available!');
      console.log('Run Node.js with --enable-sharedarraybuffer-per-context flag if needed.');
      return;
    }

    console.log(`SharedArrayBuffer available: ${sabAvailable}`);
    console.log('');

    const dataSizes = [1024, 4096, 16384, 65536, 262144]; // 1KB - 256KB
    const iterations = 1000;
    const slotSize = 512 * 1024; // 512KB per slot
    const metadataSize = 16;
    const slotCount = 8;

    // Create shared buffer
    const sharedBuffer = new SharedArrayBuffer(slotSize * slotCount);

    const results = [];

    for (const size of dataSizes) {
      console.log(`--- Data size: ${(size / 1024).toFixed(0)} KB ---`);

      // Generate test data
      const testData = new Uint8Array(size);
      for (let i = 0; i < size; i++) {
        testData[i] = i % 256;
      }

      // === SharedArrayBuffer test ===
      const sharedWorker = new Worker(__filename, {
        workerData: {
          buffer: sharedBuffer,
          slotSize,
          metadataSize,
          useShared: true,
        },
      });

      await new Promise((resolve) => {
        sharedWorker.once('message', (msg) => {
          if (msg.type === 'READY') resolve();
        });
      });

      // Write test data to shared memory
      const slotIndex = 0;
      const dataOffset = slotIndex * slotSize + metadataSize;
      const sharedView = new Uint8Array(sharedBuffer, dataOffset, size);
      sharedView.set(testData);

      const sharedStart = performance.now();
      for (let i = 0; i < iterations; i++) {
        await new Promise((resolve) => {
          const handler = (msg) => {
            if (msg.type === 'DONE') {
              sharedWorker.off('message', handler);
              resolve();
            }
          };
          sharedWorker.on('message', handler);
          sharedWorker.postMessage({ type: 'TASK', slotIndex, dataLength: size });
        });
      }
      const sharedTime = performance.now() - sharedStart;

      await sharedWorker.terminate();

      // === postMessage test ===
      const msgWorker = new Worker(__filename, {
        workerData: { useShared: false },
      });

      await new Promise((resolve) => {
        msgWorker.once('message', (msg) => {
          if (msg.type === 'READY') resolve();
        });
      });

      const msgStart = performance.now();
      for (let i = 0; i < iterations; i++) {
        await new Promise((resolve) => {
          const handler = (msg) => {
            if (msg.type === 'DONE') {
              msgWorker.off('message', handler);
              resolve();
            }
          };
          msgWorker.on('message', handler);
          // Note: This triggers structured clone of the data
          msgWorker.postMessage({ type: 'TASK', data: testData });
        });
      }
      const msgTime = performance.now() - msgStart;

      await msgWorker.terminate();

      // Calculate metrics
      const sharedOpsPerSec = Math.round(iterations / sharedTime * 1000);
      const msgOpsPerSec = Math.round(iterations / msgTime * 1000);
      const speedup = msgTime / sharedTime;

      console.log(`  SharedArrayBuffer: ${sharedTime.toFixed(0)}ms (${sharedOpsPerSec.toLocaleString()} ops/sec)`);
      console.log(`  postMessage:       ${msgTime.toFixed(0)}ms (${msgOpsPerSec.toLocaleString()} ops/sec)`);
      console.log(`  Speedup:           ${speedup.toFixed(2)}x`);
      console.log('');

      results.push({
        sizeKB: size / 1024,
        sharedTimeMs: Math.round(sharedTime),
        msgTimeMs: Math.round(msgTime),
        sharedOpsPerSec,
        msgOpsPerSec,
        speedup: parseFloat(speedup.toFixed(2)),
      });
    }

    // Summary
    console.log('=== Summary ===');
    console.log('');
    console.log('| Size | SharedArray | postMessage | Speedup |');
    console.log('|------|-------------|-------------|---------|');
    for (const r of results) {
      console.log(`| ${r.sizeKB} KB | ${r.sharedOpsPerSec} ops/s | ${r.msgOpsPerSec} ops/s | ${r.speedup}x |`);
    }

    // Export results
    const output = {
      timestamp: new Date().toISOString(),
      iterations,
      results,
    };

    console.log('');
    console.log('JSON Results:');
    console.log(JSON.stringify(output, null, 2));
  }

  runBenchmark().catch(console.error);
}
