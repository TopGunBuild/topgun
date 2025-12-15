/**
 * Mixed Workload Scenario for TopGun Server
 *
 * Realistic production-like load with mixed read/write operations.
 * - 150 VUs for 10 minutes
 * - 70% readers (subscribe and receive updates)
 * - 30% writers (generate data continuously)
 * - Tests end-to-end latency and sustained throughput
 *
 * Run:
 *   k6 run tests/k6/scenarios/mixed-workload.js -e JWT_TOKEN=<token>
 *
 * Debug mode:
 *   k6 run tests/k6/scenarios/mixed-workload.js -e JWT_TOKEN=<token> --vus 15 --duration 1m
 */

import ws from 'k6/ws';
import { check, sleep } from 'k6';
import { Counter, Trend, Rate, Gauge } from 'k6/metrics';
import {
  TopGunClient,
  createMessageHandler,
  connectionTime,
  authTime,
  authSuccess,
  errors,
} from '../lib/topgun-client.js';

// Test configuration
export const options = {
  stages: [
    { duration: '1m', target: 150 },   // Ramp up
    { duration: '8m', target: 150 },   // Sustained load
    { duration: '1m', target: 0 },     // Ramp down
  ],
  thresholds: {
    // End-to-end latency < 100ms p95
    e2e_latency: ['p(95)<100'],
    // Write latency < 50ms p95
    write_latency: ['p(95)<50'],
    // Error rate < 2%
    error_rate: ['rate<0.02'],
    // Auth success
    topgun_auth_success: ['rate>0.98'],
    // Throughput - expect >500 ops/sec sustained
    total_operations: ['count>300000'], // ~500 ops/s * 600s
  },
};

// Custom metrics for this scenario
const e2eLatency = new Trend('e2e_latency', true);
const writeLatencyMetric = new Trend('write_latency', true);
const readLatency = new Trend('read_latency', true);
const totalOperations = new Counter('total_operations');
const writeOperations = new Counter('write_operations');
const readOperations = new Counter('read_operations');
const errorRate = new Rate('error_rate');
const activeReaders = new Gauge('active_readers');
const activeWriters = new Gauge('active_writers');
const throughputPerSecond = new Trend('throughput_per_second', true);

// Configuration from environment
const WS_URL = __ENV.WS_URL || 'ws://localhost:8080';
const JWT_TOKEN = __ENV.JWT_TOKEN || null;
const READER_PERCENTAGE = parseInt(__ENV.READER_PERCENTAGE || '70');
const WRITE_RATE = parseInt(__ENV.WRITE_RATE || '10'); // Writes per second per writer
const MAPS_COUNT = parseInt(__ENV.MAPS_COUNT || '20'); // Number of maps to use

// Generate map names
const MAPS = Array.from({ length: MAPS_COUNT }, (_, i) => `k6-mixed-map-${i}`);

/**
 * Generate auth token for VU
 */
function getAuthToken(vuId) {
  if (JWT_TOKEN) {
    return JWT_TOKEN;
  }

  const header = JSON.stringify({ alg: 'HS256', typ: 'JWT' });
  const payload = JSON.stringify({
    userId: `k6-mixed-${vuId}`,
    roles: ['USER', 'ADMIN'],
    sub: `k6-mixed-${vuId}`,
    iat: Math.floor(Date.now() / 1000),
    exp: Math.floor(Date.now() / 1000) + 3600,
  });

  const b64 = (s) => {
    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
    let result = '';
    const bytes = [];
    for (let i = 0; i < s.length; i++) {
      bytes.push(s.charCodeAt(i));
    }
    for (let i = 0; i < bytes.length; i += 3) {
      const b1 = bytes[i];
      const b2 = bytes[i + 1] || 0;
      const b3 = bytes[i + 2] || 0;
      result += chars[b1 >> 2];
      result += chars[((b1 & 3) << 4) | (b2 >> 4)];
      result += i + 1 < bytes.length ? chars[((b2 & 15) << 2) | (b3 >> 6)] : '';
      result += i + 2 < bytes.length ? chars[b3 & 63] : '';
    }
    return result;
  };

  return `${b64(header)}.${b64(payload)}.mock-signature`;
}

/**
 * Determine role for this VU
 */
function getRole(vuId) {
  // Use modulo to distribute roles
  return (vuId % 100) < READER_PERCENTAGE ? 'reader' : 'writer';
}

/**
 * Main test function - runs for each VU
 */
export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-mixed-vu${vuId}-iter${iterationId}`;
  const role = getRole(vuId);

  let authenticated = false;
  let client = null;
  let opsThisSecond = 0;
  let secondStart = Date.now();

  // For readers
  let subscriptionCount = 0;
  const pendingSubscriptions = new Map();

  // For writers
  let writeCounter = 0;
  const pendingWrites = new Map();

  const connectStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - connectStart);

        if (role === 'reader') {
          activeReaders.add(1);
          setupReader();
        } else {
          activeWriters.add(1);
          setupWriter();
        }

        // Common heartbeat
        scheduleHeartbeat();
      },

      onAuthError: () => {
        errors.add(1);
        errorRate.add(1);
        socket.close();
      },

      onQueryResponse: (msg) => {
        const queryId = msg.payload?.queryId;

        // Track subscription setup latency
        if (queryId && pendingSubscriptions.has(queryId)) {
          const subStart = pendingSubscriptions.get(queryId);
          readLatency.add(Date.now() - subStart);
          pendingSubscriptions.delete(queryId);
          subscriptionCount++;
        }

        // Track e2e latency for updates
        const records = msg.payload?.records || [];
        records.forEach((record) => {
          readOperations.add(1);
          totalOperations.add(1);
          trackOps();

          const value = record?.value;
          if (value && value.sentAt) {
            const latency = Date.now() - value.sentAt;
            if (latency > 0 && latency < 10000) {
              e2eLatency.add(latency);
            }
          }
        });

        errorRate.add(0);
      },

      onOpAck: (msg) => {
        const lastId = msg.payload?.lastId;
        if (lastId && pendingWrites.has(lastId)) {
          const sendTime = pendingWrites.get(lastId);
          writeLatencyMetric.add(Date.now() - sendTime);
          pendingWrites.delete(lastId);
          errorRate.add(0);
        }
      },

      onPong: () => {
        // Connection alive
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', (e) => {
      errors.add(1);
      errorRate.add(1);
    });

    socket.on('close', () => {
      if (role === 'reader') {
        activeReaders.add(-1);
      } else {
        activeWriters.add(-1);
      }
    });

    /**
     * Track operations per second
     */
    function trackOps() {
      opsThisSecond++;
      const now = Date.now();
      if (now - secondStart >= 1000) {
        if (opsThisSecond > 0) {
          throughputPerSecond.add(opsThisSecond);
        }
        opsThisSecond = 0;
        secondStart = now;
      }
    }

    /**
     * Setup reader behavior
     */
    function setupReader() {
      // Subscribe to multiple maps (random selection)
      const mapsToSubscribe = 3 + Math.floor(Math.random() * 5); // 3-7 maps
      const selectedMaps = new Set();

      while (selectedMaps.size < mapsToSubscribe) {
        selectedMaps.add(MAPS[Math.floor(Math.random() * MAPS.length)]);
      }

      selectedMaps.forEach((mapName) => {
        const subStart = Date.now();
        const queryId = client.subscribe(mapName, {});
        pendingSubscriptions.set(queryId, subStart);
      });

      // Readers occasionally do single reads too
      scheduleOccasionalReads();
    }

    /**
     * Schedule occasional read operations for readers
     */
    function scheduleOccasionalReads() {
      function doRead() {
        if (!authenticated) return;

        // Re-subscribe to a random map (simulates refresh)
        const mapName = MAPS[Math.floor(Math.random() * MAPS.length)];
        const subStart = Date.now();
        const queryId = client.subscribe(mapName, {});
        pendingSubscriptions.set(queryId, subStart);

        // Schedule next read (every 5-15 seconds)
        const delay = 5000 + Math.random() * 10000;
        socket.setTimeout(doRead, delay);
      }

      // Start after initial subscriptions settle
      socket.setTimeout(doRead, 2000);
    }

    /**
     * Setup writer behavior
     */
    function setupWriter() {
      const intervalMs = 1000 / WRITE_RATE;

      function doWrite() {
        if (!authenticated) return;

        writeCounter++;

        // Write to a random map
        const mapName = MAPS[Math.floor(Math.random() * MAPS.length)];
        const key = `writer-${vuId}-${writeCounter}`;
        const sentAt = Date.now();

        const opId = client.putBatch([
          {
            mapName: mapName,
            key: key,
            value: {
              writerVu: vuId,
              writeNum: writeCounter,
              sentAt: sentAt,
              payload: generatePayload(),
            },
          },
        ]);

        pendingWrites.set(opId, sentAt);
        writeOperations.add(1);
        totalOperations.add(1);
        trackOps();

        // Clean up old pending writes
        const now = Date.now();
        pendingWrites.forEach((time, id) => {
          if (now - time > 5000) {
            errorRate.add(1);
            pendingWrites.delete(id);
          }
        });

        // Schedule next write
        socket.setTimeout(doWrite, intervalMs);
      }

      doWrite();
    }

    /**
     * Generate realistic payload
     */
    function generatePayload() {
      return {
        id: Math.random().toString(36).substring(7),
        timestamp: Date.now(),
        type: ['create', 'update', 'action'][Math.floor(Math.random() * 3)],
        data: {
          field1: Math.random() * 1000,
          field2: `value-${Math.random().toString(36).substring(7)}`,
          field3: Math.random() > 0.5,
          nested: {
            a: Math.random(),
            b: Math.random(),
          },
        },
      };
    }

    /**
     * Schedule heartbeat
     */
    function scheduleHeartbeat() {
      function doPing() {
        if (authenticated) {
          client.ping();
          socket.setTimeout(doPing, 10000);
        }
      }
      socket.setTimeout(doPing, 10000);
    }

    // Run for test duration with safety margin
    socket.setTimeout(function () {
      socket.close();
    }, 12 * 60 * 1000); // 12 minutes safety
  });

  // Check connection was successful
  const connected = check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });

  if (!connected) {
    errorRate.add(1);
  }

  // Keep iteration alive for full test duration
  sleep(600); // 10 minutes
}

/**
 * Setup function
 */
export function setup() {
  const writerCount = Math.floor(150 * (100 - READER_PERCENTAGE) / 100);
  const readerCount = 150 - writerCount;

  console.log('='.repeat(60));
  console.log('Mixed Workload Test');
  console.log('='.repeat(60));
  console.log(`Target: ${WS_URL}`);
  console.log(`Duration: 10 minutes (1m ramp up, 8m sustained, 1m ramp down)`);
  console.log(`Peak VUs: 150`);
  console.log(`Reader/Writer ratio: ${READER_PERCENTAGE}/${100 - READER_PERCENTAGE}`);
  console.log(`Readers: ~${readerCount}, Writers: ~${writerCount}`);
  console.log(`Write rate per writer: ${WRITE_RATE} ops/sec`);
  console.log(`Maps: ${MAPS_COUNT}`);
  console.log(`Expected throughput: ~${writerCount * WRITE_RATE} writes/sec`);
  console.log('');

  if (!JWT_TOKEN) {
    console.warn('WARNING: No JWT_TOKEN provided. Using mock tokens.');
    console.warn('Run: pnpm test:k6:token');
  }

  return { startTime: Date.now() };
}

/**
 * Teardown function
 */
export function teardown(data) {
  const duration = (Date.now() - data.startTime) / 1000;
  console.log('');
  console.log('='.repeat(60));
  console.log(`Test completed in ${duration.toFixed(2)}s`);
  console.log('='.repeat(60));
}

/**
 * Handle summary
 */
export function handleSummary(data) {
  const duration = 600; // 10 minutes
  const totalOps = data.metrics.total_operations?.values?.count || 0;

  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'mixed-workload',
    wsUrl: WS_URL,
    config: {
      peakVus: 150,
      duration: '10m',
      readerPercentage: READER_PERCENTAGE,
      writerPercentage: 100 - READER_PERCENTAGE,
      writeRatePerWriter: WRITE_RATE,
      mapsCount: MAPS_COUNT,
    },
    metrics: {
      e2eLatency: {
        avg: data.metrics.e2e_latency?.values?.avg || 0,
        p95: data.metrics.e2e_latency?.values['p(95)'] || 0,
        p99: data.metrics.e2e_latency?.values['p(99)'] || 0,
        max: data.metrics.e2e_latency?.values?.max || 0,
      },
      writeLatency: {
        avg: data.metrics.write_latency?.values?.avg || 0,
        p95: data.metrics.write_latency?.values['p(95)'] || 0,
        p99: data.metrics.write_latency?.values['p(99)'] || 0,
      },
      readLatency: {
        avg: data.metrics.read_latency?.values?.avg || 0,
        p95: data.metrics.read_latency?.values['p(95)'] || 0,
      },
      throughput: {
        totalOperations: totalOps,
        writeOperations: data.metrics.write_operations?.values?.count || 0,
        readOperations: data.metrics.read_operations?.values?.count || 0,
        avgOpsPerSecond: totalOps / duration,
      },
      errorRate: data.metrics.error_rate?.values?.rate || 0,
      authSuccessRate: data.metrics.topgun_auth_success?.values?.rate || 0,
    },
    thresholds: data.thresholds || {},
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    'tests/k6/results/mixed-workload-summary.json': JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
