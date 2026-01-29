/**
 * JSON Fallback Integration Test
 *
 * Verifies that the server correctly handles JSON-encoded messages
 * when MessagePack deserialization fails. This is critical for k6 integration
 * as k6 scripts use JSON instead of MessagePack.
 */

import { ServerCoordinator, ServerFactory } from '@topgunbuild/server';
import { deserialize } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';
import WebSocket from 'ws';

const JWT_SECRET = 'test-e2e-secret';

describe('JSON Fallback Protocol', () => {
  let server: ServerCoordinator;
  let serverUrl: string;

  beforeAll(async () => {
    server = ServerFactory.create({
      port: 0,
      nodeId: 'json-test-server',
      host: 'localhost',
      clusterPort: 0,
      metricsPort: 0,
      peers: [],
      jwtSecret: JWT_SECRET,
    });
    await server.ready();
    serverUrl = `ws://localhost:${server.port}`;
  });

  afterAll(async () => {
    await server.shutdown();
  });

  function createToken(userId: string, roles: string[] = ['ADMIN']): string {
    return jwt.sign({ userId, roles, sub: userId }, JWT_SECRET, { expiresIn: '1h' });
  }

  /**
   * Helper to create a JSON-only WebSocket client
   */
  function createJsonClient(): Promise<{
    ws: WebSocket;
    messages: any[];
    send: (msg: any) => void;
    waitFor: (type: string, timeout?: number) => Promise<any>;
    close: () => void;
  }> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(serverUrl);
      ws.binaryType = 'arraybuffer';
      const messages: any[] = [];
      const waiters = new Map<string, { resolve: (v: any) => void; timer: NodeJS.Timeout }>();

      ws.on('open', () => {
        resolve({
          ws,
          messages,
          send: (msg: any) => {
            // Send as plain JSON string (not MessagePack)
            ws.send(JSON.stringify(msg));
          },
          waitFor: (type: string, timeout = 5000) => {
            const existing = messages.find(m => m.type === type);
            if (existing) return Promise.resolve(existing);

            return new Promise((res, rej) => {
              const timer = setTimeout(() => {
                waiters.delete(type);
                rej(new Error(`Timeout waiting for ${type}`));
              }, timeout);
              waiters.set(type, { resolve: res, timer });
            });
          },
          close: () => {
            for (const { timer } of waiters.values()) clearTimeout(timer);
            ws.close();
          },
        });
      });

      ws.on('message', (data: ArrayBuffer | Buffer) => {
        try {
          const buf = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
          const msg = deserialize(buf as Uint8Array) as any;
          messages.push(msg);

          const waiter = waiters.get(msg.type);
          if (waiter) {
            clearTimeout(waiter.timer);
            waiters.delete(msg.type);
            waiter.resolve(msg);
          }
        } catch (err) {
          console.error('Failed to parse response:', err);
        }
      });

      ws.on('error', reject);
    });
  }

  it('should accept JSON-encoded AUTH message', async () => {
    const client = await createJsonClient();

    try {
      // Wait for AUTH_REQUIRED (server sends as MessagePack)
      await client.waitFor('AUTH_REQUIRED', 3000);

      // Send AUTH as plain JSON
      const token = createToken('json-user-1');
      client.send({ type: 'AUTH', token });

      // Should receive AUTH_ACK
      const ack = await client.waitFor('AUTH_ACK', 3000);
      expect(ack.type).toBe('AUTH_ACK');
    } finally {
      client.close();
    }
  });

  it('should accept JSON-encoded CLIENT_OP (PUT)', async () => {
    const client = await createJsonClient();

    try {
      await client.waitFor('AUTH_REQUIRED', 3000);
      client.send({ type: 'AUTH', token: createToken('json-user-2', ['ADMIN']) });
      await client.waitFor('AUTH_ACK', 3000);

      // Send CLIENT_OP as JSON
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-json-1',
          mapName: 'json-test-map',
          opType: 'PUT',
          key: 'json-key-1',
          record: {
            value: { name: 'JSON Test', count: 42 },
            timestamp: {
              millis: Date.now(),
              counter: 0,
              nodeId: 'json-client',
            },
          },
        },
      });

      // Wait a bit for processing
      await new Promise(r => setTimeout(r, 500));

      // Verify by subscribing (also via JSON)
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'json-query-1',
          mapName: 'json-test-map',
          query: {},
        },
      });

      const resp = await client.waitFor('QUERY_RESP', 5000);
      expect(resp.payload.results).toBeDefined();
      expect(resp.payload.results.length).toBeGreaterThanOrEqual(1);

      const found = resp.payload.results.find((r: any) => r.key === 'json-key-1');
      expect(found).toBeDefined();
      expect(found.value.name).toBe('JSON Test');
    } finally {
      client.close();
    }
  });

  it('should accept JSON-encoded QUERY_SUB', async () => {
    const client = await createJsonClient();

    try {
      await client.waitFor('AUTH_REQUIRED', 3000);
      client.send({ type: 'AUTH', token: createToken('json-user-3', ['ADMIN']) });
      await client.waitFor('AUTH_ACK', 3000);

      // First add some data to query
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'op-for-query',
          mapName: 'query-test-map',
          opType: 'PUT',
          key: 'test-key',
          record: {
            value: { status: 'active', name: 'Test' },
            timestamp: {
              millis: Date.now(),
              counter: 0,
              nodeId: 'json-client-3',
            },
          },
        },
      });

      await new Promise(r => setTimeout(r, 200));

      // Subscribe via JSON
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'json-sub-1',
          mapName: 'query-test-map',
          query: {},
        },
      });

      const resp = await client.waitFor('QUERY_RESP', 5000);
      expect(resp.type).toBe('QUERY_RESP');
      expect(resp.payload.queryId).toBe('json-sub-1');
    } finally {
      client.close();
    }
  });

  it('should accept JSON-encoded OP_BATCH', async () => {
    const client = await createJsonClient();

    try {
      await client.waitFor('AUTH_REQUIRED', 3000);
      client.send({ type: 'AUTH', token: createToken('json-user-4', ['ADMIN']) });
      await client.waitFor('AUTH_ACK', 3000);

      const now = Date.now();

      // Send batch via JSON
      client.send({
        type: 'OP_BATCH',
        payload: {
          ops: [
            {
              id: 'batch-1',
              mapName: 'batch-test-map',
              opType: 'PUT',
              key: 'batch-key-1',
              record: {
                value: { item: 1 },
                timestamp: { millis: now, counter: 0, nodeId: 'batch-client' },
              },
            },
            {
              id: 'batch-2',
              mapName: 'batch-test-map',
              opType: 'PUT',
              key: 'batch-key-2',
              record: {
                value: { item: 2 },
                timestamp: { millis: now + 1, counter: 0, nodeId: 'batch-client' },
              },
            },
          ],
        },
      });

      // Should receive OP_ACK
      const ack = await client.waitFor('OP_ACK', 5000);
      expect(ack.type).toBe('OP_ACK');
      expect(ack.payload.lastId).toBe('batch-2');
    } finally {
      client.close();
    }
  });

  it('should accept JSON-encoded PING', async () => {
    const client = await createJsonClient();

    try {
      await client.waitFor('AUTH_REQUIRED', 3000);
      client.send({ type: 'AUTH', token: createToken('json-user-5') });
      await client.waitFor('AUTH_ACK', 3000);

      const pingTime = Date.now();
      client.send({ type: 'PING', timestamp: pingTime });

      const pong = await client.waitFor('PONG', 3000);
      expect(pong.type).toBe('PONG');
      expect(pong.timestamp).toBe(pingTime);
      expect(pong.serverTime).toBeDefined();
    } finally {
      client.close();
    }
  });

  it('should reject malformed JSON', async () => {
    const client = await createJsonClient();

    try {
      await client.waitFor('AUTH_REQUIRED', 3000);

      // Send malformed data (neither valid MessagePack nor JSON)
      client.ws.send('this is not valid json {{{');

      // Connection should be closed with protocol error
      await new Promise<void>((resolve) => {
        client.ws.on('close', (code) => {
          expect(code).toBe(1002); // Protocol Error
          resolve();
        });

        // Timeout if not closed
        setTimeout(() => resolve(), 2000);
      });
    } finally {
      if (client.ws.readyState === WebSocket.OPEN) {
        client.close();
      }
    }
  });
});
