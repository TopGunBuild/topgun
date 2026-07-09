/**
 * Integration: server-issued device identity against a REAL JWT-secured Rust server.
 *
 * This is the coverage that the mocked client unit tests could not provide: the mock
 * WebSocket never validates a JWT, so it could not catch the original C2 regression
 * where a token-less client presented an empty-token AUTH that a real JWT server
 * AUTH_FAILs and tears down. Here the real server does JWT validation.
 *
 * The fix: a token-less client presents its device credential on a dedicated
 * DEVICE_HELLO frame (not AUTH). The JWT Phase-1 loop silently drops any non-AUTH
 * frame, so the connection survives and the "connect first, supply a token later"
 * flow (Case 3) still works.
 */

import WebSocket from 'ws';
import { TopGunClient, SyncState } from '@topgunbuild/client';
import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  MemoryStorageAdapter,
  waitForSync,
} from './helpers';

async function waitForState(
  client: TopGunClient,
  state: SyncState,
  timeoutMs = 20_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (client.getConnectionState() === state) return;
    await waitForSync(100);
  }
  throw new Error(`Timed out waiting for ${state}; last state = ${client.getConnectionState()}`);
}

describe('Integration: device identity vs a real JWT server', () => {
  let cleanup: () => Promise<void>;
  let port: number;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
  });

  afterAll(async () => {
    await cleanup();
  });

  // AC6b — the C2 regression, end-to-end with the real client + real JWT server.
  test('token-less TopGunClient survives against a JWT server and authenticates after setAuthToken (Case 3)', async () => {
    // No auth configured: the client is token-less at connect time. Before the fix it
    // would present AUTH{token:''}, get AUTH_FAIL'd, and be torn down. Now it presents
    // a DEVICE_HELLO (dropped by the JWT server) and parks awaiting a token.
    const client = new TopGunClient({
      serverUrl: `ws://localhost:${port}/ws`,
      storage: new MemoryStorageAdapter(),
      backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
    });

    try {
      // The connection must NOT be torn down. The JWT server sends AUTH_REQUIRED on
      // connect and the token-less client parks in AUTHENTICATING (not DISCONNECTED).
      await waitForState(client, SyncState.AUTHENTICATING);

      // Give any (wrongly) pending AUTH_FAIL/teardown a chance to surface — the state
      // must remain AUTHENTICATING, never bounce to DISCONNECTED/BACKOFF.
      await waitForSync(500);
      expect(client.getConnectionState()).toBe(SyncState.AUTHENTICATING);

      // Case 3: supplying a valid token later authenticates on the SAME connection.
      client.setAuthToken(createTestToken('device-user', ['USER']));
      await waitForState(client, SyncState.CONNECTED);
      expect(client.getConnectionState()).toBe(SyncState.CONNECTED);
    } finally {
      await client.close();
    }
  });

  // Server-side guarantee: a DEVICE_HELLO to a JWT server is NOT an auth attempt — it
  // is silently dropped in Phase 1 (never AUTH_FAIL), and a valid AUTH still works on
  // the same connection afterward.
  test('DEVICE_HELLO to a JWT server is not AUTH_FAILed and does not poison the connection', async () => {
    const client = await createRustTestClient(port, { autoAuth: false });
    try {
      await client.waitForMessage('AUTH_REQUIRED');

      // Present a device credential — a non-AUTH frame. The JWT server must drop it,
      // NOT respond with AUTH_FAIL and NOT close the socket.
      client.send({ type: 'DEVICE_HELLO' });

      await expect(client.waitForMessage('AUTH_FAIL', 800)).rejects.toBeDefined();
      expect(client.ws.readyState).toBe(WebSocket.OPEN);

      // The connection is still healthy: a subsequent valid AUTH authenticates.
      client.send({ type: 'AUTH', token: createTestToken('device-user-2', ['USER']) });
      const ack = await client.waitForMessage('AUTH_ACK');
      expect(ack.type).toBe('AUTH_ACK');
    } finally {
      client.close();
    }
  });
});
