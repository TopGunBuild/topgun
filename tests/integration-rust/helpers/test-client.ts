/**
 * Standalone test client for Rust server integration tests.
 *
 * This is a copy adapted from tests/e2e/helpers/index.ts with the following changes:
 * - No imports from @topgunbuild/server (avoids transitive TS server dependency)
 * - No MemoryStorageAdapter dependency
 * - No BATCH parsing for inbound messages (Rust server sends individual MsgPack frames)
 * - Sends individual MsgPack messages (not BATCH) to the Rust server
 * - Connects to ws://localhost:${port}/ws (Rust server mounts handler at /ws)
 */

import { serialize, deserialize } from '@topgunbuild/core';
import * as jwt from 'jsonwebtoken';
import WebSocket from 'ws';

const JWT_SECRET = 'test-e2e-secret';

export interface TestClient {
  ws: WebSocket;
  nodeId: string;
  messages: any[];
  isAuthenticated: boolean;
  send: (message: any) => void;
  waitForMessage: (type: string, timeout?: number) => Promise<any>;
  close: () => void;
  startHeartbeat: () => void;
  stopHeartbeat: () => void;
}

/**
 * Creates a JWT token for testing against the Rust server.
 * The Rust test server uses "test-e2e-secret" as the JWT secret.
 */
export function createTestToken(userId: string, roles: string[] = ['USER']): string {
  return jwt.sign(
    { sub: userId, roles },
    JWT_SECRET,
    { expiresIn: '1h' }
  );
}

/**
 * Creates a test client connected to the Rust server.
 *
 * The Rust server sends individual MsgPack binary frames per WebSocket message
 * (no BATCH wrapping on outbound). This client does not attempt BATCH parsing.
 * Inbound messages are individual MsgPack-encoded frames, decoded with deserialize().
 *
 * Auto-authentication is enabled by default: when the server sends AUTH_REQUIRED,
 * the client responds with an AUTH message containing a valid JWT token.
 */
export async function createTestClient(
  serverUrl: string,
  options: {
    nodeId?: string;
    autoAuth?: boolean;
    userId?: string;
    roles?: string[];
  } = {}
): Promise<TestClient> {
  const nodeId =
    options.nodeId ||
    `test-client-${Date.now()}-${Math.random().toString(36).slice(2)}`;

  const messages: any[] = [];
  let isAuthenticated = false;
  const resolvers: Map<
    string,
    { resolve: (value: any) => void; timeout: ReturnType<typeof setTimeout> }
  > = new Map();
  let heartbeatInterval: ReturnType<typeof setInterval> | null = null;

  return new Promise((resolve, reject) => {
    const ws = new WebSocket(serverUrl);
    ws.binaryType = 'arraybuffer';

    const client: TestClient = {
      ws,
      nodeId,
      messages,
      isAuthenticated,

      send: (message: any) => {
        if (ws.readyState === WebSocket.OPEN) {
          // Send individual MsgPack frames — the Rust server parses each
          // WebSocket binary message as one MsgPack-encoded Message.
          ws.send(serialize(message));
        }
      },

      waitForMessage: (type: string, timeout = 5000): Promise<any> => {
        // Return immediately if the message was already received
        const existing = messages.find((m) => m.type === type);
        if (existing) {
          return Promise.resolve(existing);
        }

        return new Promise((res, rej) => {
          const timer = setTimeout(() => {
            resolvers.delete(type);
            rej(new Error(`Timeout waiting for message type: ${type}`));
          }, timeout);

          resolvers.set(type, { resolve: res, timeout: timer });
        });
      },

      startHeartbeat: () => {
        if (heartbeatInterval) return;
        // Send PING every 10 seconds so the server does not close the connection
        heartbeatInterval = setInterval(() => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(serialize({ type: 'PING', timestamp: Date.now() }));
          }
        }, 10000);
      },

      stopHeartbeat: () => {
        if (heartbeatInterval) {
          clearInterval(heartbeatInterval);
          heartbeatInterval = null;
        }
      },

      close: () => {
        if (heartbeatInterval) {
          clearInterval(heartbeatInterval);
          heartbeatInterval = null;
        }
        ws.close();
        for (const { timeout } of resolvers.values()) {
          clearTimeout(timeout);
        }
        resolvers.clear();
      },
    };

    ws.on('message', (data: ArrayBuffer | Buffer) => {
      try {
        const buf = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
        // Each WebSocket frame from the Rust server is one MsgPack-encoded message.
        const message = deserialize(buf as Uint8Array) as {
          type: string;
          [key: string]: any;
        };

        messages.push(message);

        // Notify any waiters for this message type
        const resolver = resolvers.get(message.type);
        if (resolver) {
          clearTimeout(resolver.timeout);
          resolvers.delete(message.type);
          resolver.resolve(message);
        }

        // Auto-authenticate when the server challenges the connection
        if (message.type === 'AUTH_REQUIRED' && options.autoAuth !== false) {
          const token = createTestToken(
            options.userId || nodeId,
            options.roles || ['USER']
          );
          client.send({ type: 'AUTH', token });
        }

        if (message.type === 'AUTH_ACK') {
          client.isAuthenticated = true;
          // Keep the connection alive after successful authentication
          client.startHeartbeat();
        }
      } catch (err) {
        console.error('Failed to parse message:', err);
      }
    });

    ws.on('error', (err) => {
      reject(err);
    });

    ws.on('close', () => {
      client.stopHeartbeat();
    });

    // Resolve with the client object as soon as the TCP connection is open.
    // Auth handshake happens asynchronously via the message handler above.
    ws.on('open', () => {
      resolve(client);
    });
  });
}

/**
 * Waits for a brief period to allow async sync to propagate.
 */
export function waitForSync(ms = 100): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Polls a condition until it is true or a timeout is reached.
 */
export async function waitUntil(
  condition: () => boolean | Promise<boolean>,
  timeout = 5000,
  interval = 50
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await condition()) {
      return;
    }
    await waitForSync(interval);
  }
  throw new Error('waitUntil timeout');
}

/**
 * Creates an LWW record for testing. Mirrors the e2e helper pattern.
 * The record contains a value, an HLC timestamp, and implicitly uses
 * the LWW (Last-Write-Wins) merge strategy.
 */
export function createLWWRecord<T>(value: T, nodeId = 'test-node'): any {
  return {
    value,
    timestamp: {
      millis: Date.now(),
      counter: 0,
      nodeId,
    },
  };
}

/**
 * Creates an OR record for testing. Mirrors the e2e helper pattern.
 * The record contains a value, an HLC timestamp, and a unique tag
 * used for Observed-Remove semantics (OR_ADD / OR_REMOVE).
 */
export function createORRecord<T>(value: T, nodeId = 'test-node'): any {
  const ts = Date.now();
  return {
    value,
    timestamp: {
      millis: ts,
      counter: 0,
      nodeId,
    },
    tag: `${nodeId}-${ts}-0`,
  };
}
