/**
 * Phase 3 Benchmark Test
 *
 * Measures the impact of Phase 3 native optimizations:
 * - Native xxHash64 for Merkle tree hashing
 * - SharedArrayBuffer for worker communication
 * - Overall throughput improvement
 *
 * Run:
 *   k6 run tests/k6/scenarios/phase3-benchmark.js -e JWT_TOKEN=<token>
 *
 * Or with pnpm:
 *   pnpm test:k6:phase3
 */

import ws from 'k6/ws';
import { check, sleep } from 'k6';
import { Counter, Trend, Rate, Gauge } from 'k6/metrics';
import {
  TopGunClient,
  createMessageHandler,
  connectionTime,
  errors,
} from '../lib/topgun-client.js';
import {
  getWsUrl,
  getAuthToken,
  getConfig,
  getResultsPath,
} from '../lib/config.js';

// Configuration
const WS_URL = getWsUrl();
const BATCH_SIZE = getConfig('BATCH_SIZE', 10);
const OPS_PER_SECOND = getConfig('OPS_PER_SECOND', 25); // Higher for Phase 3

// Test stages: aggressive ramp-up to measure peak throughput
export const options = {
  scenarios: {
    phase3_benchmark: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '10s', target: 30 },   // Warm up
        { duration: '30s', target: 75 },   // Sustained load
        { duration: '30s', target: 150 },  // High load
        { duration: '30s', target: 200 },  // Peak load - target 25K+ ops/sec
        { duration: '10s', target: 75 },   // Cooldown
      ],
      gracefulRampDown: '5s',
    },
  },
  thresholds: {
    // Phase 3 targets (more aggressive than Phase 2)
    write_latency: ['p(95)<40', 'p(99)<80'],  // Tighter latency
    write_error_rate: ['rate<0.003'],          // <0.3% error rate
    topgun_auth_success: ['rate>0.99'],
  },
};

// Custom metrics
const writeLatency = new Trend('write_latency', true);
const writeOpsTotal = new Counter('write_ops_total');
const writeOpsAcked = new Counter('write_ops_acked');
const writeErrorRate = new Rate('write_error_rate');
const currentThroughput = new Gauge('current_throughput');
const batchLatency = new Trend('batch_latency', true);

// Global tracking
let globalOpsCount = 0;
let lastReportTime = Date.now();

function generatePayload(vuId, opNum) {
  // Generate payload that will benefit from native hash
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    phase3Test: true,
    nativeOptimized: true,
    data: `phase3-native-${Math.random().toString(36).substring(7)}`,
    nested: {
      field1: Math.random() * 1000,
      field2: `value-${opNum}`,
      tags: ['phase3', 'native', 'xxhash64', 'sharedmemory'],
      metadata: {
        created: new Date().toISOString(),
        index: opNum,
      },
    },
    // Larger payload to benefit from SharedArrayBuffer
    largeField: 'x'.repeat(200),
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-phase3-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-phase3', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);
        scheduleWrites();
        scheduleHeartbeat();
      },

      onAuthError: () => {
        errors.add(1);
        socket.close();
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingOps.has(lastId)) {
          const sendTime = pendingOps.get(lastId);
          const latency = Date.now() - sendTime;
          writeLatency.add(latency);
          batchLatency.add(latency);
          writeOpsAcked.add(BATCH_SIZE);
          for (let i = 0; i < BATCH_SIZE; i++) {
            writeErrorRate.add(0);
          }
          pendingOps.delete(lastId);
          globalOpsCount += BATCH_SIZE;
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', () => {
      errors.add(1);
      writeErrorRate.add(1);
    });

    socket.on('close', () => {
      if (authenticated) {
        pendingOps.forEach(() => {
          for (let i = 0; i < BATCH_SIZE; i++) {
            writeErrorRate.add(1);
          }
        });
      }
    });

    function scheduleWrites() {
      const intervalMs = 1000 / (OPS_PER_SECOND / BATCH_SIZE);

      function doWrite() {
        if (!authenticated) return;

        // Track throughput
        const now = Date.now();
        if (now - lastReportTime >= 1000) {
          const elapsed = (now - lastReportTime) / 1000;
          const throughput = globalOpsCount / elapsed;
          currentThroughput.add(throughput);
          globalOpsCount = 0;
          lastReportTime = now;
        }

        // Create batch
        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          operations.push({
            mapName: `k6-phase3-${vuId % 10}`,
            key: `key-${vuId}-${opCounter}`,
            value: generatePayload(vuId, opCounter),
          });
        }

        const sendTime = Date.now();
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        writeOpsTotal.add(BATCH_SIZE);

        // Cleanup stale
        const timeout = 5000;
        pendingOps.forEach((time, id) => {
          if (now - time > timeout) {
            for (let i = 0; i < BATCH_SIZE; i++) {
              writeErrorRate.add(1);
            }
            pendingOps.delete(id);
          }
        });

        socket.setTimeout(doWrite, intervalMs);
      }

      doWrite();
    }

    function scheduleHeartbeat() {
      function doPing() {
        if (authenticated) {
          client.ping();
          socket.setTimeout(doPing, 10000);
        }
      }
      socket.setTimeout(doPing, 10000);
    }

    // Session timeout
    const maxSessionTime = 110 * 1000;
    socket.setTimeout(() => {
      authenticated = false;
      socket.setTimeout(() => socket.close(), 2000);
    }, maxSessionTime);
  });

  check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });
}

export function setup() {
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║             TOPGUN PHASE 3 NATIVE OPTIMIZATION BENCHMARK         ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  Measuring impact of:                                            ║');
  console.log('║  • Native xxHash64 for Merkle tree hashing                       ║');
  console.log('║  • SharedArrayBuffer for worker communication                    ║');
  console.log('║  • Integrated Phase 1+2+3 optimizations                          ║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');
  console.log('');
  console.log(`Target: ${WS_URL}`);
  console.log(`Batch size: ${BATCH_SIZE}`);
  console.log(`Ops per second per VU: ${OPS_PER_SECOND}`);
  console.log('Stages: 10 VUs → 30 → 75 → 150 → 200 → 75 VUs');
  console.log('');

  return { startTime: Date.now() };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log(`Test completed in ${duration.toFixed(2)}s`);
}

export function handleSummary(data) {
  const totalOps = data.metrics.write_ops_total?.values?.count || 0;
  const ackedOps = data.metrics.write_ops_acked?.values?.count || 0;
  const testDuration = 110; // Approximate test duration
  const avgThroughput = ackedOps / testDuration;
  const peakThroughput = data.metrics.current_throughput?.values?.max || avgThroughput;

  const p50 = data.metrics.write_latency?.values?.med || 0;
  const p95 = data.metrics.write_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.write_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.write_latency?.values?.max || 0;
  const errorRate = data.metrics.write_error_rate?.values?.rate || 0;

  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                   PHASE 3 BENCHMARK RESULTS                      ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  THROUGHPUT                                                      ║');
  console.log(`║    Total Operations Sent:      ${totalOps.toLocaleString().padStart(12)}                  ║`);
  console.log(`║    Total Operations Acked:     ${ackedOps.toLocaleString().padStart(12)}                  ║`);
  console.log(`║    Average Throughput:         ${avgThroughput.toFixed(0).padStart(12)} ops/sec          ║`);
  console.log(`║    Peak Throughput:            ${peakThroughput.toFixed(0).padStart(12)} ops/sec          ║`);
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  LATENCY                                                         ║');
  console.log(`║    p50:                        ${p50.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    p95:                        ${p95.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    p99:                        ${p99.toFixed(1).padStart(12)} ms                ║`);
  console.log(`║    max:                        ${maxLatency.toFixed(1).padStart(12)} ms                ║`);
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log('║  RELIABILITY                                                     ║');
  console.log(`║    Error Rate:                 ${(errorRate * 100).toFixed(3).padStart(12)}%               ║`);
  console.log(`║    Success Rate:               ${((1 - errorRate) * 100).toFixed(3).padStart(12)}%               ║`);
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  // Phase 3 targets assessment
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                    PHASE 3 TARGET ASSESSMENT                     ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');

  // Phase 3 targets: 25K ops/sec, p99 <80ms, error <0.3%
  const throughputTarget = avgThroughput >= 20000;  // 20K minimum
  const peakTarget = peakThroughput >= 25000;       // 25K peak
  const latencyTarget = p99 < 80;
  const errorTarget = errorRate < 0.003;

  console.log(`║  Avg Throughput ≥20K ops/sec:  ${throughputTarget ? '✅ PASS' : '❌ FAIL'}  (${avgThroughput.toFixed(0)} ops/sec)`.padEnd(69) + '║');
  console.log(`║  Peak Throughput ≥25K ops/sec: ${peakTarget ? '✅ PASS' : '❌ FAIL'}  (${peakThroughput.toFixed(0)} ops/sec)`.padEnd(69) + '║');
  console.log(`║  p99 Latency <80ms:            ${latencyTarget ? '✅ PASS' : '❌ FAIL'}  (${p99.toFixed(1)}ms)`.padEnd(69) + '║');
  console.log(`║  Error Rate <0.3%:             ${errorTarget ? '✅ PASS' : '❌ FAIL'}  (${(errorRate * 100).toFixed(3)}%)`.padEnd(69) + '║');

  const allPass = throughputTarget && peakTarget && latencyTarget && errorTarget;
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  console.log(`║  Overall:                      ${allPass ? '✅ ALL TARGETS MET' : '⚠️  TARGETS NOT MET'}`.padEnd(69) + '║');
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  // Comparison with Phase 2 baseline
  console.log('');
  console.log('╔══════════════════════════════════════════════════════════════════╗');
  console.log('║                PHASE 2 → PHASE 3 IMPROVEMENT                     ║');
  console.log('╠══════════════════════════════════════════════════════════════════╣');
  const phase2Baseline = 18000; // Phase 2 result
  const improvement = ((avgThroughput - phase2Baseline) / phase2Baseline * 100);
  console.log(`║  Phase 2 Baseline:             ${phase2Baseline.toLocaleString().padStart(12)} ops/sec          ║`);
  console.log(`║  Phase 3 Result:               ${avgThroughput.toFixed(0).padStart(12)} ops/sec          ║`);
  console.log(`║  Improvement:                  ${improvement > 0 ? '+' : ''}${improvement.toFixed(1).padStart(11)}%                ║`);
  console.log('╚══════════════════════════════════════════════════════════════════╝');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'phase3-benchmark',
    version: 'phase3-native-optimized',
    config: {
      batchSize: BATCH_SIZE,
      opsPerSecond: OPS_PER_SECOND,
      wsUrl: WS_URL,
    },
    results: {
      throughput: {
        totalOpsSent: totalOps,
        totalOpsAcked: ackedOps,
        avgOpsPerSec: avgThroughput,
        peakOpsPerSec: peakThroughput,
      },
      latency: {
        p50: p50,
        p95: p95,
        p99: p99,
        max: maxLatency,
      },
      reliability: {
        errorRate: errorRate,
        successRate: 1 - errorRate,
      },
    },
    targets: {
      avgThroughput: { target: 20000, actual: avgThroughput, pass: throughputTarget },
      peakThroughput: { target: 25000, actual: peakThroughput, pass: peakTarget },
      latencyP99: { target: 80, actual: p99, pass: latencyTarget },
      errorRate: { target: 0.003, actual: errorRate, pass: errorTarget },
      allPass: allPass,
    },
    comparison: {
      phase2Baseline: phase2Baseline,
      phase3Result: avgThroughput,
      improvementPercent: improvement,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('phase3-benchmark.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
