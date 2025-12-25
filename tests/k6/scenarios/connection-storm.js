/**
 * Connection Storm Scenario for TopGun Server
 *
 * Stress test for concurrent connections with ramping VUs.
 * - Ramping: 0 → 50 → 150 → 300 VUs over 3 minutes
 * - Each VU: connects, authenticates, holds connection, disconnects
 * - Tests server's ability to handle connection spikes
 *
 * Run:
 *   k6 run tests/k6/scenarios/connection-storm.js -e JWT_TOKEN=<token>
 *
 * Debug mode (lower load):
 *   k6 run tests/k6/scenarios/connection-storm.js -e JWT_TOKEN=<token> --vus 10 --duration 30s
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
const HOLD_TIME_MS = getConfig('HOLD_TIME', 5000);

// Test configuration with ramping stages
export const options = {
  stages: [
    { duration: '30s', target: 50 },    // Ramp up to 50 VUs
    { duration: '30s', target: 100 },   // Ramp to 100
    { duration: '30s', target: 150 },   // Ramp to 150
    { duration: '30s', target: 200 },   // Ramp to 200
    { duration: '30s', target: 300 },   // Ramp to 300
    { duration: '30s', target: 300 },   // Hold at 300
  ],
  thresholds: {
    // Connection errors < 5%
    connection_error_rate: ['rate<0.05'],
    // p95 connection time < 500ms
    topgun_connection_time: ['p(95)<500'],
    // Auth success > 95%
    topgun_auth_success: ['rate>0.95'],
    // Total errors should be minimal
    topgun_errors: ['rate<0.05'],
  },
};

// Custom metrics for this scenario
const connectionErrorRate = new Rate('connection_error_rate');
const connectionsPerSecond = new Counter('connections_per_second');
const activeConnections = new Gauge('active_connections');
const connectionAttempts = new Counter('connection_attempts');
const successfulConnections = new Counter('successful_connections');
const connectionHoldTime = new Trend('connection_hold_time', true);

/**
 * Main test function - runs for each VU
 */
export default function () {
  const vuId = __VU;
  const iterationId = __ITER;
  const nodeId = `k6-storm-vu${vuId}-iter${iterationId}`;

  connectionAttempts.add(1);
  connectionsPerSecond.add(1);

  let authenticated = false;
  let holdStartTime = null;
  let client = null;

  const connectStart = Date.now();

  const res = ws.connect(WS_URL, {}, function (socket) {
    activeConnections.add(1);
    client = new TopGunClient(socket, nodeId);

    const handleMessage = createMessageHandler(client, {
      onAuthRequired: () => {
        const token = getAuthToken(vuId, 'k6-storm', ['USER']);
        client.authenticate(token);
      },

      onAuthAck: () => {
        authenticated = true;
        connectionTime.add(Date.now() - connectStart);
        connectionErrorRate.add(0); // Success
        successfulConnections.add(1);
        holdStartTime = Date.now();

        // Start periodic pings to keep connection alive
        client.ping();
      },

      onAuthError: () => {
        connectionErrorRate.add(1); // Failure
        errors.add(1);
        socket.close();
      },

      onPong: () => {
        // Keep connection alive with periodic pings
        if (holdStartTime && Date.now() - holdStartTime < HOLD_TIME_MS) {
          socket.setTimeout(() => {
            if (socket.readyState === 1) { // OPEN
              client.ping();
            }
          }, 1000);
        }
      },
    });

    socket.on('binaryMessage', handleMessage);

    socket.on('error', () => {
      connectionErrorRate.add(1);
      errors.add(1);
    });

    socket.on('close', () => {
      activeConnections.add(-1);
      if (holdStartTime) {
        connectionHoldTime.add(Date.now() - holdStartTime);
      }
    });

    // Hold connection for specified time, then close
    socket.setTimeout(function () {
      socket.close();
    }, HOLD_TIME_MS);
  });

  // Check connection was successful
  const connected = check(res, {
    'WebSocket connected': (r) => r && r.status === 101,
  });

  if (!connected) {
    connectionErrorRate.add(1);
    errors.add(1);
  }

  // Small delay between iterations to spread load
  sleep(Math.random() * 0.5);
}

/**
 * Setup function
 */
export function setup() {
  logTestHeader('Connection Storm Test', {
    'Target': WS_URL,
    'Ramping': '0 → 50 → 150 → 300 VUs over 3 minutes',
    'Hold time': `${HOLD_TIME_MS}ms per connection`,
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
    scenario: 'connection-storm',
    wsUrl: WS_URL,
    metrics: {
      connectionTime: {
        avg: data.metrics.topgun_connection_time?.values?.avg || 0,
        p95: data.metrics.topgun_connection_time?.values['p(95)'] || 0,
        p99: data.metrics.topgun_connection_time?.values['p(99)'] || 0,
      },
      connectionErrorRate: data.metrics.connection_error_rate?.values?.rate || 0,
      authSuccessRate: data.metrics.topgun_auth_success?.values?.rate || 0,
      totalConnectionAttempts: data.metrics.connection_attempts?.values?.count || 0,
      successfulConnections: data.metrics.successful_connections?.values?.count || 0,
      totalErrors: data.metrics.topgun_errors?.values?.count || 0,
    },
    thresholds: data.thresholds || {},
  };

  return {
    stdout: textSummary(data, { indent: ' ', enableColors: true }),
    [getResultsPath('connection-storm-summary.json')]: JSON.stringify(summary, null, 2),
  };
}

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.1/index.js';
