/**
 * TopGun Cluster Failover Test (Phase 4.5 Task 07)
 *
 * Tests cluster resilience during node failure:
 * - Measures failover recovery time (<5 seconds target)
 * - Tracks operations before/during/after failover
 * - Verifies client reconnection to healthy nodes
 *
 * Run:
 *   k6 run tests/k6/scenarios/cluster-failover.js -e JWT_TOKEN=<token>
 *
 * Or with pnpm:
 *   pnpm test:k6:cluster:failover
 *
 * Note: Node failure simulation requires external trigger (docker/script)
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
  getAuthToken,
  getConfig,
  getResultsPath,
} from '../lib/config.js';

// Cluster Configuration
const CLUSTER_NODES = [
  __ENV.NODE1_URL || 'ws://localhost:8081',
  __ENV.NODE2_URL || 'ws://localhost:8082',
  __ENV.NODE3_URL || 'ws://localhost:8083',
];

const BATCH_SIZE = getConfig('BATCH_SIZE', 5);
const MAX_VUS = getConfig('MAX_VUS', 50);
const FAILOVER_TIME_MS = getConfig('FAILOVER_TIME', 30000); // Node fails at 30s

// Test configuration - 2 minute test with failover at 30s
export const options = {
  scenarios: {
    failover_test: {
      executor: 'constant-vus',
      vus: MAX_VUS,
      duration: '120s',
    },
  },
  thresholds: {
    'failover_recovery_time': ['p(95)<5000'],  // Recovery < 5s
    'failover_success_rate': ['rate>0.95'],     // 95% success despite failover
    'failover_ops_total': ['count>50000'],      // 50K ops in 2 min with failover
  },
};

// Failover-specific metrics
const failoverOpsTotal = new Counter('failover_ops_total');
const failoverOpsAcked = new Counter('failover_ops_acked');
const failoverLatency = new Trend('failover_latency', true);
const failoverSuccessRate = new Rate('failover_success_rate');
const failoverRecoveryTime = new Trend('failover_recovery_time', true);
const nodeReconnections = new Counter('node_reconnections');
const activeConnections = new Gauge('active_connections');

// Track test phase
const testStartTime = Date.now();
let failoverTriggered = false;

/**
 * Get current test phase
 */
function getTestPhase() {
  const elapsed = Date.now() - testStartTime;
  if (elapsed < FAILOVER_TIME_MS) return 'pre-failover';
  if (elapsed < FAILOVER_TIME_MS + 10000) return 'during-failover';
  return 'post-failover';
}

function generatePayload(vuId, opNum) {
  return {
    vuId,
    opNum,
    timestamp: Date.now(),
    phase: getTestPhase(),
    data: `failover-test-${Math.random().toString(36).substring(7)}`,
  };
}

/**
 * Try to connect to an available node
 */
function getAvailableNode(excludeIndex) {
  const indices = [0, 1, 2].filter((i) => i !== excludeIndex);
  // Shuffle for load distribution
  for (let i = indices.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [indices[i], indices[j]] = [indices[j], indices[i]];
  }
  return indices[0];
}

export default function () {
  const vuId = __VU;
  const iterationId = __ITER;

  // Distribute VUs across cluster nodes
  let nodeIndex = vuId % CLUSTER_NODES.length;
  let wsUrl = CLUSTER_NODES[nodeIndex];
  const nodeId = `k6-failover-vu${vuId}-iter${iterationId}`;

  let authenticated = false;
  let client = null;
  let opCounter = 0;
  let pendingOps = new Map();
  let reconnectAttempts = 0;
  let lastDisconnectTime = 0;
  const sessionStart = Date.now();

  function connectToNode(url, onReady) {
    const res = ws.connect(url, {}, function (socket) {
      client = new TopGunClient(socket, nodeId);
      activeConnections.add(1);

      const handleMessage = createMessageHandler(client, {
        onAuthRequired: () => {
          const token = getAuthToken(vuId, 'k6-failover', ['USER', 'ADMIN']);
          client.authenticate(token);
        },

        onAuthAck: () => {
          authenticated = true;
          connectionTime.add(Date.now() - sessionStart);

          // If this is a reconnection, record recovery time
          if (lastDisconnectTime > 0) {
            const recoveryTime = Date.now() - lastDisconnectTime;
            failoverRecoveryTime.add(recoveryTime);
            nodeReconnections.add(1);
            lastDisconnectTime = 0;
          }

          if (onReady) onReady();
          scheduleWrites();
        },

        onAuthError: () => {
          errors.add(1);
          failoverSuccessRate.add(0);
          socket.close();
        },

        onOpAck: (msg) => {
          const lastId = msg.payload?.lastId;
          if (lastId && pendingOps.has(lastId)) {
            const sendTime = pendingOps.get(lastId);
            const latency = Date.now() - sendTime;
            failoverLatency.add(latency);
            failoverOpsAcked.add(BATCH_SIZE);
            for (let i = 0; i < BATCH_SIZE; i++) {
              failoverSuccessRate.add(1);
            }
            pendingOps.delete(lastId);
          }
        },
      });

      socket.on('binaryMessage', handleMessage);

      socket.on('error', (e) => {
        errors.add(1);
        failoverSuccessRate.add(0);

        // Try reconnecting to another node
        if (authenticated && getTestPhase() !== 'pre-failover') {
          lastDisconnectTime = Date.now();
          authenticated = false;
          reconnectAttempts++;

          if (reconnectAttempts < 5) {
            // Switch to another node
            nodeIndex = getAvailableNode(nodeIndex);
            wsUrl = CLUSTER_NODES[nodeIndex];

            socket.setTimeout(() => {
              connectToNode(wsUrl, null);
            }, 500 * reconnectAttempts);
          }
        }
      });

      socket.on('close', () => {
        activeConnections.add(-1);
        if (authenticated && pendingOps.size > 0) {
          // Mark pending ops as failed
          pendingOps.forEach(() => {
            for (let i = 0; i < BATCH_SIZE; i++) {
              failoverSuccessRate.add(0);
            }
          });
          pendingOps.clear();
        }
      });

      function scheduleWrites() {
        const intervalMs = 100; // 10 batches/sec per VU

        function doWrite() {
          if (!authenticated) return;

          const operations = [];
          for (let i = 0; i < BATCH_SIZE; i++) {
            opCounter++;
            operations.push({
              mapName: `k6-failover-${vuId % 10}`,
              key: `key-${vuId}-${opCounter % 50}`,
              value: generatePayload(vuId, opCounter),
            });
          }

          const sendTime = Date.now();
          const lastOpId = client.putBatch(operations);
          pendingOps.set(lastOpId, sendTime);
          failoverOpsTotal.add(BATCH_SIZE);

          // Cleanup stale pending ops
          const now = Date.now();
          const timeout = 10000; // Longer timeout during failover
          pendingOps.forEach((time, id) => {
            if (now - time > timeout) {
              for (let i = 0; i < BATCH_SIZE; i++) {
                failoverSuccessRate.add(0);
              }
              pendingOps.delete(id);
            }
          });

          socket.setTimeout(doWrite, intervalMs);
        }

        doWrite();
      }

      // Keep session alive for test duration
      const maxSessionTime = 115 * 1000;
      socket.setTimeout(() => {
        authenticated = false;
        socket.setTimeout(() => socket.close(), 2000);
      }, maxSessionTime);
    });

    check(res, {
      'WebSocket connected': (r) => r && r.status === 101,
    });
  }

  connectToNode(wsUrl, null);
}

export function setup() {
  console.log('');
  console.log('========================================================');
  console.log('         TOPGUN CLUSTER FAILOVER TEST                   ');
  console.log('========================================================');
  console.log('');
  console.log('Cluster Nodes:');
  CLUSTER_NODES.forEach((url, i) => {
    console.log(`  Node ${i + 1}: ${url}`);
  });
  console.log('');
  console.log(`  Max VUs:            ${MAX_VUS}`);
  console.log(`  Batch size:         ${BATCH_SIZE}`);
  console.log(`  Failover at:        ${FAILOVER_TIME_MS / 1000}s`);
  console.log('');
  console.log('  IMPORTANT: To trigger failover, stop node-2 at 30s:');
  console.log('    docker stop topgun-node-2');
  console.log('');

  return { startTime: Date.now(), failoverTime: FAILOVER_TIME_MS };
}

export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log(`Test completed in ${duration.toFixed(2)}s`);
  console.log('');
  console.log('  Restart stopped node with:');
  console.log('    docker start topgun-node-2');
}

export function handleSummary(data) {
  const totalOps = data.metrics.failover_ops_total?.values?.count || 0;
  const ackedOps = data.metrics.failover_ops_acked?.values?.count || 0;
  const testDuration = 120;
  const avgThroughput = ackedOps / testDuration;

  const p50 = data.metrics.failover_latency?.values?.med || 0;
  const p95 = data.metrics.failover_latency?.values['p(95)'] || 0;
  const p99 = data.metrics.failover_latency?.values['p(99)'] || 0;
  const maxLatency = data.metrics.failover_latency?.values?.max || 0;
  const successRate = data.metrics.failover_success_rate?.values?.rate || 0;

  const recoveryP50 =
    data.metrics.failover_recovery_time?.values?.med || 0;
  const recoveryP95 =
    data.metrics.failover_recovery_time?.values['p(95)'] || 0;
  const reconnections =
    data.metrics.node_reconnections?.values?.count || 0;

  console.log('');
  console.log('========================================================');
  console.log('           CLUSTER FAILOVER RESULTS                     ');
  console.log('========================================================');
  console.log('');
  console.log('  THROUGHPUT');
  console.log(`    Total Operations Sent:      ${totalOps.toLocaleString()}`);
  console.log(`    Total Operations Acked:     ${ackedOps.toLocaleString()}`);
  console.log(`    Average Throughput:         ${avgThroughput.toFixed(0)} ops/sec`);
  console.log('');
  console.log('  LATENCY');
  console.log(`    p50:                        ${p50.toFixed(1)} ms`);
  console.log(`    p95:                        ${p95.toFixed(1)} ms`);
  console.log(`    p99:                        ${p99.toFixed(1)} ms`);
  console.log(`    max:                        ${maxLatency.toFixed(1)} ms`);
  console.log('');
  console.log('  FAILOVER METRICS');
  console.log(`    Reconnections:              ${reconnections}`);
  console.log(`    Recovery Time p50:          ${recoveryP50.toFixed(1)} ms`);
  console.log(`    Recovery Time p95:          ${recoveryP95.toFixed(1)} ms`);
  console.log('');
  console.log('  RELIABILITY');
  console.log(`    Success Rate:               ${(successRate * 100).toFixed(2)}%`);
  console.log('');

  // Performance assessment
  const recoveryTarget = recoveryP95 < 5000;
  const successTarget = successRate >= 0.95;
  const throughputTarget = totalOps >= 50000;

  console.log('  TARGETS');
  console.log(
    `    Recovery < 5s:    ${recoveryTarget ? 'PASS' : 'FAIL'} (${recoveryP95.toFixed(0)}ms)`
  );
  console.log(
    `    95% success:      ${successTarget ? 'PASS' : 'FAIL'} (${(successRate * 100).toFixed(2)}%)`
  );
  console.log(
    `    50K+ ops:         ${throughputTarget ? 'PASS' : 'FAIL'} (${totalOps})`
  );
  console.log('');

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'cluster-failover',
    clusterNodes: CLUSTER_NODES,
    failoverTimeMs: FAILOVER_TIME_MS,
    results: {
      throughput: {
        totalOpsSent: totalOps,
        totalOpsAcked: ackedOps,
        avgOpsPerSec: avgThroughput,
      },
      latency: { p50, p95, p99, max: maxLatency },
      failover: {
        reconnections,
        recoveryTimeP50: recoveryP50,
        recoveryTimeP95: recoveryP95,
      },
      reliability: { successRate },
    },
    targets: {
      recoveryTime: { target: 5000, actual: recoveryP95, pass: recoveryTarget },
      successRate: { target: 0.95, actual: successRate, pass: successTarget },
      throughput: { target: 50000, actual: totalOps, pass: throughputTarget },
      allPass: recoveryTarget && successTarget && throughputTarget,
    },
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('cluster-failover.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
