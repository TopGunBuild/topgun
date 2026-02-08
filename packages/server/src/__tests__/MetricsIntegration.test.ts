import { ServerCoordinator, ServerFactory } from '../';
import { WebSocket } from 'ws';
import { register } from 'prom-client';
import * as http from 'http';
import * as jwt from 'jsonwebtoken';
import { serialize } from '@topgunbuild/core';
import { pollUntil } from './utils/test-helpers';

describe('Metrics Integration', () => {
  let server: ServerCoordinator;
  const METRICS_PORT = 19090;
  const SERVER_PORT = 13000;
  const CLUSTER_PORT = 13001;
  const JWT_SECRET = 'topgun-secret-dev';

  beforeAll(async () => {
    register.clear();
    server = ServerFactory.create({
      port: SERVER_PORT,
      nodeId: 'test-node',
      host: 'localhost',
      clusterPort: CLUSTER_PORT,
      metricsPort: METRICS_PORT,
      peers: [],
      securityPolicies: [{
        role: 'USER',
        mapNamePattern: 'metrics-test',
        actions: ['READ', 'PUT', 'REMOVE']
      }]
    });
    await server.ready();
  });

  afterAll(async () => {
    await server.shutdown();
    register.clear();
  });

  const fetchMetrics = (): Promise<string> => {
    return new Promise((resolve, reject) => {
      http.get(`http://localhost:${METRICS_PORT}/metrics`, (res) => {
        let data = '';
        res.on('data', chunk => data += chunk);
        res.on('end', () => resolve(data));
      }).on('error', reject);
    });
  };

  test('should expose metrics endpoint', async () => {
    const metrics = await fetchMetrics();
    expect(metrics).toContain('topgun_process_start_time_seconds');
  });

  test('should update metrics on client connection', async () => {
    // Initial check
    let metrics = await fetchMetrics();
    // Might be 0 or more if previous tests leaked, but we cleared register so 0
    // Note: register.clear() clears the default registry, but ServerCoordinator 
    // creates its own MetricsService which creates a NEW Registry. 
    // But prom-client's default registry is a singleton if not carefully managed.
    // MetricsService uses `new Registry()`, so it's isolated. Good.
    
    // Connect a client
    const ws = new WebSocket(`ws://localhost:${server.port}`);
    await new Promise<void>(resolve => ws.on('open', () => resolve()));

    // Wait for connected_clients metric to show 1
    await pollUntil(
      async () => {
        const m = await fetchMetrics();
        return m.includes('topgun_connected_clients 1');
      },
      { timeoutMs: 5000, intervalMs: 50, description: 'connected_clients metric updated to 1' }
    );

    metrics = await fetchMetrics();
    expect(metrics).toContain('topgun_connected_clients 1');

    ws.close();

    // Wait for connected_clients metric to return to 0
    await pollUntil(
      async () => {
        const m = await fetchMetrics();
        return m.includes('topgun_connected_clients 0');
      },
      { timeoutMs: 5000, intervalMs: 50, description: 'connected_clients metric updated to 0' }
    );

    metrics = await fetchMetrics();
    expect(metrics).toContain('topgun_connected_clients 0');
  });

  test('should update metrics on operations', async () => {
    const ws = new WebSocket(`ws://localhost:${server.port}`);
    await new Promise<void>(resolve => ws.on('open', () => resolve()));
    
    // 1. Authenticate
    const token = jwt.sign({ userId: 'test-user', roles: ['USER'] }, JWT_SECRET);
    ws.send(serialize({
        type: 'AUTH',
        token
    }));

    // Wait for auth ack
    await new Promise<void>((resolve) => {
        ws.once('message', (data) => {
            resolve();
        });
    });

    // 2. Send Client Op (PUT)
    const op = {
        opType: 'set',
        mapName: 'metrics-test',
        key: 'key1',
        record: {
            value: { foo: 'bar' },
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'client' }
        }
    };

    ws.send(serialize({
        type: 'CLIENT_OP',
        payload: op
    }));

    // Wait for PUT operation metric to appear
    await pollUntil(
      async () => {
        const m = await fetchMetrics();
        return m.includes('topgun_ops_total{type="PUT",map="metrics-test"} 1');
      },
      { timeoutMs: 5000, intervalMs: 50, description: 'PUT operation metric recorded' }
    );

    // 3. Verify Metrics
    const metrics = await fetchMetrics();
    expect(metrics).toContain('topgun_ops_total{type="PUT",map="metrics-test"} 1');

    // 4. Send Subscribe (SUBSCRIBE)
    ws.send(serialize({
        type: 'QUERY_SUB',
        payload: {
            queryId: 'q1',
            mapName: 'metrics-test',
            query: {}
        }
    }));

    // Wait for SUBSCRIBE operation metric to appear
    await pollUntil(
      async () => {
        const m = await fetchMetrics();
        return m.includes('topgun_ops_total{type="SUBSCRIBE",map="metrics-test"} 1');
      },
      { timeoutMs: 5000, intervalMs: 50, description: 'SUBSCRIBE operation metric recorded' }
    );

    const metrics2 = await fetchMetrics();
    expect(metrics2).toContain('topgun_ops_total{type="SUBSCRIBE",map="metrics-test"} 1');

    ws.close();
  });
});

