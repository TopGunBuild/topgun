/**
 * Integration tests for the POST /sync HTTP endpoint.
 *
 * Exercises the endpoint directly with raw MsgPack requests (not HttpSyncProvider)
 * to validate the handler's decode/dispatch/response pipeline end-to-end.
 */

import { serialize, deserialize } from '@topgunbuild/core';
import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  waitForSync,
} from './helpers';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Sends a POST /sync request with the given MsgPack-serialized body.
 * Returns the raw Response so each test can inspect status, headers, and body.
 */
async function postSync(port: number, body: Uint8Array | Buffer): Promise<Response> {
  return fetch(`http://127.0.0.1:${port}/sync`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/msgpack' },
    body: Buffer.from(body),
  });
}

/** Builds a minimal valid HttpSyncRequest body (no operations). */
function emptyRequest(): Uint8Array {
  return serialize({
    clientId: 'test-client-http',
    clientHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node' },
  });
}

/** Builds an HttpSyncRequest body with a single PUT operation. */
function requestWithOp(opId: string, mapName: string, key: string, value: unknown): Uint8Array {
  return serialize({
    clientId: 'test-client-http',
    clientHlc: { millis: Date.now(), counter: 0, nodeId: 'test-node' },
    operations: [
      {
        id: opId,
        mapName,
        opType: 'PUT',
        key,
        record: createLWWRecord(value),
      },
    ],
  });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Integration: HTTP Sync (POST /sync)', () => {
  test('POST /sync with empty request returns valid response', async () => {
    const { port, cleanup } = await spawnRustServer();

    try {
      const resp = await postSync(port, emptyRequest());

      expect(resp.status).toBe(200);
      expect(resp.headers.get('content-type')).toBe('application/msgpack');

      const buf = await resp.arrayBuffer();
      const decoded = deserialize<any>(new Uint8Array(buf));

      // serverHlc must be present and millis must be a positive wall-clock value.
      expect(decoded).toBeDefined();
      expect(decoded.serverHlc).toBeDefined();
      expect(typeof decoded.serverHlc.millis).toBe('number');
      expect(decoded.serverHlc.millis).toBeGreaterThan(0);
    } finally {
      await cleanup();
    }
  });

  test('POST /sync with operations returns ack', async () => {
    const { port, cleanup } = await spawnRustServer();

    try {
      const opId = `op-http-${Date.now()}`;
      const resp = await postSync(
        port,
        requestWithOp(opId, 'http-test-map', 'key-http-1', { source: 'http' }),
      );

      expect(resp.status).toBe(200);

      const buf = await resp.arrayBuffer();
      const decoded = deserialize<any>(new Uint8Array(buf));

      // ack must be present and lastId must match the submitted operation ID.
      expect(decoded.ack).toBeDefined();
      expect(decoded.ack.lastId).toBe(opId);
    } finally {
      await cleanup();
    }
  });

  test('POST /sync with malformed body returns 400', async () => {
    const { port, cleanup } = await spawnRustServer();

    try {
      // Two bytes that are not a valid MsgPack HttpSyncRequest.
      const resp = await postSync(port, Buffer.from([0xff, 0xff]));
      expect(resp.status).toBe(400);
    } finally {
      await cleanup();
    }
  });

  test('POST /sync operations are processed by CRDT pipeline', async () => {
    // Write a record via HTTP sync, then read it back via WebSocket to confirm
    // both transports share the same CRDT pipeline.
    const { port, cleanup: killServer } = await spawnRustServer();
    const client = await createRustTestClient(port, {
      nodeId: 'ws-reader',
      userId: 'user-ws-reader',
      roles: ['ADMIN'],
    });

    try {
      await client.waitForMessage('AUTH_ACK', 10_000);

      const mapName = 'http-pipeline-test';
      const key = `key-shared-${Date.now()}`;
      const opId = `op-shared-${Date.now()}`;
      const value = { writtenVia: 'http', verified: true };

      // Write via HTTP sync.
      const resp = await postSync(port, requestWithOp(opId, mapName, key, value));
      expect(resp.status).toBe(200);

      // Wait for the write to propagate through the CRDT pipeline.
      await waitForSync(300);

      // Read back via WebSocket QUERY_SUB.
      client.messages.length = 0;
      client.send({
        type: 'QUERY_SUB',
        payload: {
          queryId: 'q-http-shared',
          mapName,
          query: {},
        },
      });

      const response = await client.waitForMessage('QUERY_RESP', 5_000);
      expect(response).toBeDefined();
      expect(response.payload.queryId).toBe('q-http-shared');

      const results: any[] = response.payload.results ?? [];
      const entry = results.find((r: any) => r.key === key);
      expect(entry).toBeDefined();

      // Confirm the value written via HTTP is readable via WebSocket.
      const readValue = entry.record?.value ?? entry.value;
      expect(readValue).toEqual(value);
    } finally {
      client.close();
      await killServer();
    }
  });
});
