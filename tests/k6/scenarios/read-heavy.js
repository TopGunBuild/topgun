/**
 * Read-Heavy Scenario for TopGun Server
 *
 * Test for massive subscriptions and update propagation.
 * - 200 VUs for 3 minutes
 * - Each VU subscribes to 5 different maps
 * - Dedicated "writer" VUs (10%) generate data
 * - Measures subscription latency and update propagation time
 *
 * Run:
 *   k6 run tests/k6/scenarios/read-heavy.js -e JWT_TOKEN=<token>
 *
 * Debug mode:
 *   k6 run tests/k6/scenarios/read-heavy.js -e JWT_TOKEN=<token> --vus 20 --duration 30s
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
  logTestHeader,
  getResultsPath,
} from '../lib/config.js';

// Configuration
const WS_URL = getWsUrl();
const MAPS_PER_VU = getConfig('MAPS_PER_VU', 5);
const WRITER_PERCENTAGE = getConfig('WRITER_PERCENTAGE', 10);
const WRITES_PER_SECOND = getConfig('WRITES_PER_SECOND', 5);

// Test configuration
export const options = {
  vus: 200,
  duration: '3m',
  thresholds: {
    // Subscription setup should be fast
    subscription_latency: ['p(95)<200'],
    // Update propagation < 50ms p95
    update_propagation_time: ['p(95)<50'],
    // Error rate < 2%
    topgun_errors: ['rate<0.02'],
    // Auth success
    topgun_auth_success: ['rate>0.98'],
  },
};

// Custom metrics for this scenario
const subscriptionLatency = new Trend('subscription_latency', true);
const updatePropagationTime = new Trend('update_propagation_time', true);
const subscriptionsActive = new Gauge('subscriptions_active');
const updatesReceived = new Counter('updates_received');
const updatesSent = new Counter('updates_sent');
const subscriptionErrors = new Rate('subscription_errors');

// Shared maps for subscriptions
const SHARED_MAPS = [
  'k6-shared-map-1',
  'k6-shared-map-2',
  'k6-shared-map-3',
  'k6-shared-map-4',
  'k6-shared-map-5',
  'k6-shared-map-6',
  'k6-shared-map-7',
  'k6-shared-map-8',
  'k6-shared-map-9',
  'k6-shared-map-10',
];

/**
 * Determine if this VU should be a writer
 */
function isWriter(vuId) {
  return vuId % (100 / WRITER_PERCENTAGE) === 0;
}

/**
 * Get maps for this VU to subscribe to
 */
function getMapsForVU(vuId) {
  const maps = [];
  const startIdx = vuId % SHARED_MAPS.length;
  for (let i = 0; i < MAPS_PER_VU; i++) {
    maps.push(SHARED_MAPS[(startIdx + i) % SHARED_MAPS.length]);
  }
  return maps;
}

/**
 * Main test function - runs for each VU
 */
export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-reader-vu${vuId}-iter${iterationId}`;
  const isWriterVU = isWriter(vuId);

  let authenticated = false;
  let client = null;
  let activeSubscriptions = 0;
  const pendingSubscriptions = new Map(); // queryId -> startTime
  const sentUpdates = new Map(); // key -> sendTime (for propagation tracking)

  const connectStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-reader', ['USER', 'ADMIN']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - connectStart);

        // Subscribe to maps
        const maps = getMapsForVU(vuId);
        maps.forEach((mapName) => {
          const subStart = Date.now();
          const queryId = client.subscribe(mapName, {});
          pendingSubscriptions.set(queryId, subStart);
        });

        // If writer, start generating data
        if (isWriterVU) {
          scheduleWrites();
        }

        // Start heartbeat
        scheduleHeartbeat();
      },

      onAuthError: () => {
        errors.add(1);
        socket.close();
      },

      onQueryResponse: (msg) => {
        const queryId = msg.payload?.queryId;

        // Track subscription latency for initial response
        if (queryId && pendingSubscriptions.has(queryId)) {
          const subStart = pendingSubscriptions.get(queryId);
          subscriptionLatency.add(Date.now() - subStart);
          pendingSubscriptions.delete(queryId);
          activeSubscriptions++;
          subscriptionsActive.add(1);
          subscriptionErrors.add(0);
        }

        // Track update propagation
        const records = msg.payload?.records || [];
        records.forEach((record) => {
          updatesReceived.add(1);

          // Check if we can measure propagation time
          // Records from writers include timestamp in value
          const value = record?.value;
          if (value && value.sentAt) {
            const propagationTime = Date.now() - value.sentAt;
            if (propagationTime > 0 && propagationTime < 10000) {
              updatePropagationTime.add(propagationTime);
            }
          }
        });
      },

      onPong: () => {
        // Connection alive
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', () => {
      errors.add(1);
      subscriptionErrors.add(1);
    });

    socket.on('close', () => {
      subscriptionsActive.add(-activeSubscriptions);
    });

    /**
     * Schedule periodic writes for writer VUs
     */
    function scheduleWrites() {
      const intervalMs = 1000 / WRITES_PER_SECOND;
      let writeCounter = 0;

      function doWrite() {
        if (!authenticated) return;

        writeCounter++;

        // Write to a random shared map
        const mapName = SHARED_MAPS[Math.floor(Math.random() * SHARED_MAPS.length)];
        const key = `writer-${vuId}-${writeCounter}`;
        const sentAt = Date.now();

        client.putBatch([
          {
            mapName: mapName,
            key: key,
            value: {
              writerVu: vuId,
              writeNum: writeCounter,
              sentAt: sentAt, // For propagation time measurement
              data: `update-${Math.random().toString(36).substring(7)}`,
            },
          },
        ]);

        updatesSent.add(1);
        sentUpdates.set(key, sentAt);

        // Clean up old entries
        const now = Date.now();
        sentUpdates.forEach((time, k) => {
          if (now - time > 10000) {
            sentUpdates.delete(k);
          }
        });

        // Schedule next write
        socket.setTimeout(doWrite, intervalMs);
      }

      doWrite();
    }

    /**
     * Schedule heartbeat
     */
    function scheduleHeartbeat() {
      function doPing() {
        if (authenticated) {
          client.ping();
          socket.setTimeout(doPing, 5000);
        }
      }
      socket.setTimeout(doPing, 5000);
    }

    // Run for test duration
    socket.setTimeout(function () {
      socket.close();
    }, 4 * 60 * 1000); // 4 minutes safety
  });

  // Check connection was successful
  check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });

  // Keep iteration alive
  sleep(180); // 3 minutes
}

/**
 * Setup function
 */
export function setup() {
  logTestHeader('Read-Heavy Test', {
    'Target': WS_URL,
    'VUs': options.vus,
    'Duration': options.duration,
    'Maps per VU': MAPS_PER_VU,
    'Writer percentage': `${WRITER_PERCENTAGE}%`,
    'Writes per second per writer': WRITES_PER_SECOND,
    'Total subscriptions': options.vus * MAPS_PER_VU,
    'Writer VUs': Math.floor(options.vus * WRITER_PERCENTAGE / 100),
  });

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
  const summary = {
    timestamp: new Date().toISOString(),
    scenario: 'read-heavy',
    wsUrl: WS_URL,
    config: {
      vus: options.vus,
      duration: options.duration,
      mapsPerVu: MAPS_PER_VU,
      writerPercentage: WRITER_PERCENTAGE,
      totalSubscriptions: options.vus * MAPS_PER_VU,
    },
    metrics: {
      subscriptionLatency: {
        avg: data.metrics.subscription_latency?.values?.avg || 0,
        p95: data.metrics.subscription_latency?.values['p(95)'] || 0,
        p99: data.metrics.subscription_latency?.values['p(99)'] || 0,
      },
      updatePropagation: {
        avg: data.metrics.update_propagation_time?.values?.avg || 0,
        p95: data.metrics.update_propagation_time?.values['p(95)'] || 0,
        p99: data.metrics.update_propagation_time?.values['p(99)'] || 0,
      },
      updates: {
        sent: data.metrics.updates_sent?.values?.count || 0,
        received: data.metrics.updates_received?.values?.count || 0,
      },
      subscriptionErrorRate: data.metrics.subscription_errors?.values?.rate || 0,
      authSuccessRate: data.metrics.topgun_auth_success?.values?.rate || 0,
    },
    thresholds: data.thresholds || {},
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('read-heavy-summary.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
