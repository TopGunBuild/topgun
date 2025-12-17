/**
 * Throughput Saturation Test for TopGun Server
 *
 * Determines maximum sustainable throughput by ramping up load
 * until latency degrades or errors appear.
 *
 * Run:
 *   k6 run tests/k6/scenarios/throughput-test.js -e JWT_TOKEN=<token>
 */

import ws from 'k6/ws';
import { check } from 'k6';
import { Counter, Trend, Rate } from 'k6/metrics';
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
const BATCH_SIZE = getConfig('BATCH_SIZE', 5);

// Ramping stages to find saturation point
export const options = {
  scenarios: {
    throughput_ramp: {
      executor: 'ramping-vus',
      startVUs: 10,
      stages: [
        { duration: '20s', target: 50 },   // Warm up
        { duration: '20s', target: 100 },  // Increase
        { duration: '20s', target: 150 },  // More load
        { duration: '20s', target: 200 },  // High load
        { duration: '20s', target: 250 },  // Very high
        { duration: '20s', target: 300 },  // Peak
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    write_latency: ['p(99)<500'],  // Allow higher latency to find limits
    topgun_auth_success: ['rate>0.95'],
  },
};

// Metrics
const writeLatency = new Trend('write_latency', true);
const writeOpsTotal = new Counter('write_ops_total');
const writeOpsAcked = new Counter('write_ops_acked');
const writeErrorRate = new Rate('write_error_rate');

function generatePayload(vuId, opNum) {
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    data: `throughput-test-${Math.random().toString(36).substring(7)}`,
  };
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-throughput-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  const sessionStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-throughput', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - sessionStart);
        scheduleWrites();
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
          writeOpsAcked.add(BATCH_SIZE);
          for (let i = 0; i < BATCH_SIZE; i++) {
            writeErrorRate.add(0);
          }
          pendingOps.delete(lastId);
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
      // Send as fast as possible to find limits
      const intervalMs = 50; // 20 batches/sec per VU = 100 ops/sec per VU

      function doWrite() {
        if (!authenticated) return;

        const operations = [];
        for (let i = 0; i < BATCH_SIZE; i++) {
          opCounter++;
          operations.push({
            mapName: `k6-throughput-${vuId % 20}`,
            key: `key-${vuId}-${opCounter}`,
            value: generatePayload(vuId, opCounter),
          });
        }

        const sendTime = Date.now();
        const lastOpId = client.putBatch(operations);
        pendingOps.set(lastOpId, sendTime);
        writeOpsTotal.add(BATCH_SIZE);

        // Cleanup stale pending ops
        const now = Date.now();
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

    // Keep session alive for test duration
    const maxSessionTime = 120 * 1000; // 2 minutes max
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
  console.log('='.repeat(60));
  console.log('TopGun Throughput Saturation Test');
  console.log('='.repeat(60));
  console.log(`Target: ${WS_URL}`);
  console.log(`Batch size: ${BATCH_SIZE}`);
  console.log('Ramping: 10 → 50 → 100 → 150 → 200 → 250 → 300 VUs');
  console.log('');

  return { startTime: Date.now() };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log('='.repeat(60));
  console.log(`Test completed in ${duration.toFixed(2)}s`);
  console.log('='.repeat(60));
}

export function handleSummary(data) {
  const totalOps = data.metrics.write_ops_total?.values?.count || 0;
  const ackedOps = data.metrics.write_ops_acked?.values?.count || 0;
  const testDuration = 120; // ~2 minutes of ramping
  const avgThroughput = ackedOps / testDuration;

  console.log('');
  console.log('╔══════════════════════════════════════════════════════════╗');
  console.log('║           THROUGHPUT TEST RESULTS                        ║');
  console.log('╠══════════════════════════════════════════════════════════╣');
  console.log(`║  Total Operations Sent:    ${totalOps.toLocaleString().padStart(15)}          ║`);
  console.log(`║  Total Operations Acked:   ${ackedOps.toLocaleString().padStart(15)}          ║`);
  console.log(`║  Average Throughput:       ${avgThroughput.toFixed(0).padStart(15)} ops/sec   ║`);
  console.log('╠══════════════════════════════════════════════════════════╣');
  console.log('║  LATENCY (time to Early ACK, before in-memory write)     ║');
  console.log(`║  Write Latency p50:        ${(data.metrics.write_latency?.values?.med || 0).toFixed(0).padStart(15)} ms       ║`);
  console.log(`║  Write Latency p95:        ${(data.metrics.write_latency?.values['p(95)'] || 0).toFixed(0).padStart(15)} ms       ║`);
  console.log(`║  Write Latency p99:        ${(data.metrics.write_latency?.values['p(99)'] || 0).toFixed(0).padStart(15)} ms       ║`);
  console.log(`║  Error Rate:               ${((data.metrics.write_error_rate?.values?.rate || 0) * 100).toFixed(2).padStart(15)}%        ║`);
  console.log('╚══════════════════════════════════════════════════════════╝');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'throughput-saturation',
    results: {
      totalOpsSent: totalOps,
      totalOpsAcked: ackedOps,
      avgThroughput: avgThroughput,
      writeLatency: {
        p50: data.metrics.write_latency?.values?.med || 0,
        p95: data.metrics.write_latency?.values['p(95)'] || 0,
        p99: data.metrics.write_latency?.values['p(99)'] || 0,
        max: data.metrics.write_latency?.values?.max || 0,
      },
      errorRate: data.metrics.write_error_rate?.values?.rate || 0,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('throughput-summary.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
