/**
 * Cross-language golden fixture generator.
 *
 * This test generates MsgPack fixture files for 40+ message types that the Rust
 * integration test (cross_lang_compat.rs) reads and verifies. Each fixture is a
 * representative instance packed with msgpackr, written to packages/core-rust/tests/fixtures/.
 *
 * The JSON files are written alongside for human-readable debugging.
 */

import * as fs from 'fs';
import * as path from 'path';
import { pack } from 'msgpackr';

// Fixture output directory (relative to repo root)
const FIXTURES_DIR = path.resolve(
  __dirname,
  '../../../../packages/core-rust/tests/fixtures'
);

// Helper to write a fixture: both .msgpack (binary) and .json (human-readable)
function writeFixture(name: string, data: Record<string, unknown>): void {
  const msgpackPath = path.join(FIXTURES_DIR, `${name}.msgpack`);
  const jsonPath = path.join(FIXTURES_DIR, `${name}.json`);

  const packed = pack(data);
  fs.writeFileSync(msgpackPath, packed);
  fs.writeFileSync(jsonPath, JSON.stringify(data, null, 2));
}

// Reusable test data
const timestamp = { millis: 1700000000000, counter: 1, nodeId: 'node-1' };
const timestamp2 = { millis: 1700000000001, counter: 0, nodeId: 'node-2' };

beforeAll(() => {
  // Ensure the fixtures directory exists
  fs.mkdirSync(FIXTURES_DIR, { recursive: true });
});

describe('Cross-language fixture generation', () => {
  // =========================================================================
  // Domain 1: Base (2 messages)
  // =========================================================================

  test('AUTH', () => {
    writeFixture('AUTH', {
      type: 'AUTH',
      token: 'eyJhbGciOiJIUzI1NiJ9.test-jwt-token',
      protocolVersion: 1,
    });
  });

  test('AUTH_REQUIRED', () => {
    writeFixture('AUTH_REQUIRED', {
      type: 'AUTH_REQUIRED',
      reason: 'token expired',
    });
  });

  // =========================================================================
  // Domain 2: Sync (10 messages)
  // =========================================================================

  test('CLIENT_OP', () => {
    writeFixture('CLIENT_OP', {
      type: 'CLIENT_OP',
      payload: {
        id: 'op-1',
        mapName: 'users',
        key: 'user-1',
        opType: 'PUT',
        record: {
          value: { name: 'Alice', age: 30 },
          timestamp,
          ttlMs: 60000,
        },
      },
    });
  });

  test('OP_BATCH', () => {
    writeFixture('OP_BATCH', {
      type: 'OP_BATCH',
      payload: {
        ops: [
          {
            id: 'op-1',
            mapName: 'users',
            key: 'user-1',
            record: {
              value: 'Alice',
              timestamp,
            },
          },
          {
            id: 'op-2',
            mapName: 'users',
            key: 'user-2',
            record: {
              value: 'Bob',
              timestamp: timestamp2,
            },
          },
        ],
        writeConcern: 'MEMORY',
        timeout: 5000,
      },
    });
  });

  test('SYNC_INIT', () => {
    writeFixture('SYNC_INIT', {
      type: 'SYNC_INIT',
      mapName: 'users',
      rootHash: 12345,
      bucketHashes: { '0': 111, '1': 222, '2': 333 },
      lastSyncTimestamp: 1700000000000,
    });
  });

  test('SYNC_RESP_ROOT', () => {
    writeFixture('SYNC_RESP_ROOT', {
      type: 'SYNC_RESP_ROOT',
      payload: {
        mapName: 'users',
        rootHash: 12345,
        timestamp,
      },
    });
  });

  test('OP_ACK', () => {
    writeFixture('OP_ACK', {
      type: 'OP_ACK',
      payload: {
        lastId: 'op-1',
        achievedLevel: 'MEMORY',
        results: [
          {
            opId: 'op-1',
            success: true,
            achievedLevel: 'MEMORY',
          },
        ],
      },
    });
  });

  test('OP_REJECTED', () => {
    writeFixture('OP_REJECTED', {
      type: 'OP_REJECTED',
      payload: {
        opId: 'op-bad',
        reason: 'schema validation failed',
        code: 400,
      },
    });
  });

  test('BATCH', () => {
    writeFixture('BATCH', {
      type: 'BATCH',
      count: 3,
      data: Buffer.from([0xDE, 0xAD, 0xBE, 0xEF]),
    });
  });

  test('ORMAP_SYNC_INIT', () => {
    writeFixture('ORMAP_SYNC_INIT', {
      type: 'ORMAP_SYNC_INIT',
      mapName: 'tags',
      rootHash: 99999,
      bucketHashes: { '0': 444, '5': 555 },
    });
  });

  test('MERKLE_REQ_BUCKET', () => {
    writeFixture('MERKLE_REQ_BUCKET', {
      type: 'MERKLE_REQ_BUCKET',
      payload: {
        mapName: 'users',
        path: '0/1/2',
      },
    });
  });

  test('SYNC_RESP_BUCKETS', () => {
    writeFixture('SYNC_RESP_BUCKETS', {
      type: 'SYNC_RESP_BUCKETS',
      payload: {
        mapName: 'users',
        path: '0',
        buckets: { '0': 111, '1': 222, '2': 333 },
      },
    });
  });

  // =========================================================================
  // Domain 3: Query (4 messages)
  // =========================================================================

  test('QUERY_SUB', () => {
    writeFixture('QUERY_SUB', {
      type: 'QUERY_SUB',
      payload: {
        queryId: 'q-001',
        mapName: 'users',
        query: {
          where: { age: { $gt: 18 } },
          sort: { createdAt: 'desc' },
          limit: 50,
        },
      },
    });
  });

  test('QUERY_UNSUB', () => {
    writeFixture('QUERY_UNSUB', {
      type: 'QUERY_UNSUB',
      payload: {
        queryId: 'q-001',
      },
    });
  });

  test('QUERY_RESP', () => {
    writeFixture('QUERY_RESP', {
      type: 'QUERY_RESP',
      payload: {
        queryId: 'q-001',
        results: [
          { key: 'user-1', value: { name: 'Alice' } },
          { key: 'user-2', value: { name: 'Bob' } },
        ],
        hasMore: true,
        cursorStatus: 'valid',
        nextCursor: 'cursor-abc',
      },
    });
  });

  test('QUERY_UPDATE', () => {
    writeFixture('QUERY_UPDATE', {
      type: 'QUERY_UPDATE',
      payload: {
        queryId: 'q-001',
        key: 'user-3',
        value: { name: 'Charlie' },
        changeType: 'ENTER',
      },
    });
  });

  // =========================================================================
  // Domain 4: Search (5 messages)
  // =========================================================================

  test('SEARCH', () => {
    writeFixture('SEARCH', {
      type: 'SEARCH',
      payload: {
        requestId: 'sr-001',
        mapName: 'articles',
        query: 'rust async programming',
        options: { limit: 10, minScore: 0.5 },
      },
    });
  });

  test('SEARCH_RESP', () => {
    writeFixture('SEARCH_RESP', {
      type: 'SEARCH_RESP',
      payload: {
        requestId: 'sr-001',
        results: [
          {
            key: 'article-1',
            value: { title: 'Async Rust' },
            score: 0.95,
            matchedTerms: ['rust', 'async'],
          },
        ],
        totalCount: 42,
      },
    });
  });

  test('SEARCH_SUB', () => {
    writeFixture('SEARCH_SUB', {
      type: 'SEARCH_SUB',
      payload: {
        subscriptionId: 'ss-001',
        mapName: 'articles',
        query: 'machine learning',
        options: { limit: 20, minScore: 0.3 },
      },
    });
  });

  test('SEARCH_UPDATE', () => {
    writeFixture('SEARCH_UPDATE', {
      type: 'SEARCH_UPDATE',
      payload: {
        subscriptionId: 'ss-001',
        key: 'article-99',
        value: { title: 'New ML Paper' },
        score: 0.88,
        matchedTerms: ['machine', 'learning'],
        changeType: 'ENTER',
      },
    });
  });

  test('SEARCH_UNSUB', () => {
    writeFixture('SEARCH_UNSUB', {
      type: 'SEARCH_UNSUB',
      payload: {
        subscriptionId: 'ss-001',
      },
    });
  });

  // =========================================================================
  // Domain 5: Cluster (11 messages)
  // =========================================================================

  test('PARTITION_MAP_REQUEST', () => {
    writeFixture('PARTITION_MAP_REQUEST', {
      type: 'PARTITION_MAP_REQUEST',
      payload: { currentVersion: 5 },
    });
  });

  test('PARTITION_MAP', () => {
    writeFixture('PARTITION_MAP', {
      type: 'PARTITION_MAP',
      payload: {
        version: 7,
        partitionCount: 271,
        nodes: [
          {
            nodeId: 'node-1',
            endpoints: {
              websocket: 'ws://10.0.0.1:8080',
              http: 'http://10.0.0.1:3000',
            },
            status: 'ACTIVE',
          },
        ],
        partitions: [
          {
            partitionId: 0,
            ownerNodeId: 'node-1',
            backupNodeIds: ['node-2'],
          },
        ],
        generatedAt: 1700000000000,
      },
    });
  });

  test('CLUSTER_SUB_REGISTER', () => {
    writeFixture('CLUSTER_SUB_REGISTER', {
      type: 'CLUSTER_SUB_REGISTER',
      payload: {
        subscriptionId: 'csub-001',
        coordinatorNodeId: 'node-1',
        mapName: 'articles',
        type: 'SEARCH',
        searchQuery: 'rust',
        searchOptions: { limit: 10 },
      },
    });
  });

  test('CLUSTER_SUB_ACK', () => {
    writeFixture('CLUSTER_SUB_ACK', {
      type: 'CLUSTER_SUB_ACK',
      payload: {
        subscriptionId: 'csub-001',
        nodeId: 'node-2',
        success: true,
        initialResults: [
          { key: 'doc-1', value: 'content', score: 0.9, matchedTerms: ['rust'] },
        ],
        totalHits: 42,
      },
    });
  });

  test('CLUSTER_SUB_UPDATE', () => {
    writeFixture('CLUSTER_SUB_UPDATE', {
      type: 'CLUSTER_SUB_UPDATE',
      payload: {
        subscriptionId: 'csub-001',
        sourceNodeId: 'node-2',
        key: 'doc-5',
        value: { title: 'updated' },
        score: 0.8,
        matchedTerms: ['rust'],
        changeType: 'ENTER',
        timestamp: 1700000000001,
      },
    });
  });

  test('CLUSTER_SUB_UNREGISTER', () => {
    writeFixture('CLUSTER_SUB_UNREGISTER', {
      type: 'CLUSTER_SUB_UNREGISTER',
      payload: {
        subscriptionId: 'csub-001',
      },
    });
  });

  test('CLUSTER_SEARCH_REQ', () => {
    writeFixture('CLUSTER_SEARCH_REQ', {
      type: 'CLUSTER_SEARCH_REQ',
      payload: {
        requestId: 'csearch-001',
        mapName: 'products',
        query: 'laptop',
        options: {
          limit: 25,
          includeMatchedTerms: true,
        },
        timeoutMs: 5000,
      },
    });
  });

  test('CLUSTER_SEARCH_RESP', () => {
    writeFixture('CLUSTER_SEARCH_RESP', {
      type: 'CLUSTER_SEARCH_RESP',
      payload: {
        requestId: 'csearch-001',
        nodeId: 'node-3',
        results: [
          { key: 'prod-1', value: 'Laptop Pro', score: 0.92, matchedTerms: ['laptop'] },
        ],
        totalHits: 150,
        executionTimeMs: 23,
      },
    });
  });

  test('CLUSTER_SEARCH_SUBSCRIBE', () => {
    writeFixture('CLUSTER_SEARCH_SUBSCRIBE', {
      type: 'CLUSTER_SEARCH_SUBSCRIBE',
      payload: {
        subscriptionId: 'css-001',
        mapName: 'products',
        query: 'monitor',
        options: { limit: 5 },
      },
    });
  });

  test('CLUSTER_SEARCH_UNSUBSCRIBE', () => {
    writeFixture('CLUSTER_SEARCH_UNSUBSCRIBE', {
      type: 'CLUSTER_SEARCH_UNSUBSCRIBE',
      payload: {
        subscriptionId: 'css-001',
      },
    });
  });

  test('CLUSTER_SEARCH_UPDATE', () => {
    writeFixture('CLUSTER_SEARCH_UPDATE', {
      type: 'CLUSTER_SEARCH_UPDATE',
      payload: {
        subscriptionId: 'css-001',
        nodeId: 'node-2',
        key: 'prod-99',
        value: 'Updated Monitor',
        score: 0.85,
        matchedTerms: ['monitor'],
        changeType: 'UPDATE',
      },
    });
  });

  // =========================================================================
  // Domain 6: Messaging (18 messages)
  // =========================================================================

  test('TOPIC_SUB', () => {
    writeFixture('TOPIC_SUB', {
      type: 'TOPIC_SUB',
      payload: { topic: 'chat/room-1' },
    });
  });

  test('TOPIC_UNSUB', () => {
    writeFixture('TOPIC_UNSUB', {
      type: 'TOPIC_UNSUB',
      payload: { topic: 'chat/room-1' },
    });
  });

  test('TOPIC_PUB', () => {
    writeFixture('TOPIC_PUB', {
      type: 'TOPIC_PUB',
      payload: { topic: 'notifications', data: { text: 'hello world' } },
    });
  });

  test('TOPIC_MESSAGE', () => {
    writeFixture('TOPIC_MESSAGE', {
      type: 'TOPIC_MESSAGE',
      payload: {
        topic: 'chat/room-1',
        data: { text: 'Hi there' },
        publisherId: 'user-42',
        timestamp: 1700000000000,
      },
    });
  });

  test('LOCK_REQUEST', () => {
    writeFixture('LOCK_REQUEST', {
      type: 'LOCK_REQUEST',
      payload: { requestId: 'lock-req-1', name: 'my-lock', ttl: 30000 },
    });
  });

  test('LOCK_RELEASE', () => {
    writeFixture('LOCK_RELEASE', {
      type: 'LOCK_RELEASE',
      payload: { requestId: 'lock-req-1', name: 'my-lock', fencingToken: 7 },
    });
  });

  test('COUNTER_REQUEST', () => {
    writeFixture('COUNTER_REQUEST', {
      type: 'COUNTER_REQUEST',
      payload: { name: 'page-views' },
    });
  });

  test('COUNTER_SYNC', () => {
    writeFixture('COUNTER_SYNC', {
      type: 'COUNTER_SYNC',
      payload: {
        name: 'page-views',
        state: { p: { 'node-1': 10 }, n: {} },
      },
    });
  });

  test('PING', () => {
    writeFixture('PING', {
      type: 'PING',
      timestamp: 1700000000000,
    });
  });

  test('PONG', () => {
    writeFixture('PONG', {
      type: 'PONG',
      timestamp: 1700000000000,
      serverTime: 1700000000005,
    });
  });

  test('ENTRY_PROCESS', () => {
    writeFixture('ENTRY_PROCESS', {
      type: 'ENTRY_PROCESS',
      requestId: 'ep-1',
      mapName: 'counters',
      key: 'counter-1',
      processor: { name: 'increment', code: 'return value + 1', args: 1 },
    });
  });

  test('ENTRY_PROCESS_RESPONSE', () => {
    writeFixture('ENTRY_PROCESS_RESPONSE', {
      type: 'ENTRY_PROCESS_RESPONSE',
      requestId: 'ep-1',
      success: true,
      result: 42,
      newValue: 42,
    });
  });

  test('JOURNAL_SUBSCRIBE', () => {
    writeFixture('JOURNAL_SUBSCRIBE', {
      type: 'JOURNAL_SUBSCRIBE',
      requestId: 'jsub-1',
      fromSequence: 'seq-100',
      mapName: 'users',
      types: ['PUT', 'DELETE'],
    });
  });

  test('JOURNAL_EVENT', () => {
    writeFixture('JOURNAL_EVENT', {
      type: 'JOURNAL_EVENT',
      event: {
        event: {
          sequence: 'seq-050',
          type: 'UPDATE',
          mapName: 'orders',
          key: 'order-7',
          value: 'shipped',
          previousValue: 'pending',
          timestamp,
          nodeId: 'node-1',
        },
      },
    });
  });

  test('REGISTER_RESOLVER', () => {
    writeFixture('REGISTER_RESOLVER', {
      type: 'REGISTER_RESOLVER',
      requestId: 'rr-1',
      mapName: 'users',
      resolver: {
        name: 'custom-merge',
        code: 'if (a > b) return a; else return b;',
        priority: 50,
        keyPattern: 'user-*',
      },
    });
  });

  test('MERGE_REJECTED', () => {
    writeFixture('MERGE_REJECTED', {
      type: 'MERGE_REJECTED',
      mapName: 'users',
      key: 'user-1',
      attemptedValue: 'bad-value',
      reason: 'resolver rejected',
      timestamp,
    });
  });

  test('LIST_RESOLVERS', () => {
    writeFixture('LIST_RESOLVERS', {
      type: 'LIST_RESOLVERS',
      requestId: 'lr-1',
      mapName: 'users',
    });
  });

  test('LIST_RESOLVERS_RESPONSE', () => {
    writeFixture('LIST_RESOLVERS_RESPONSE', {
      type: 'LIST_RESOLVERS_RESPONSE',
      requestId: 'lr-1',
      resolvers: [
        { mapName: 'users', name: 'merge-a', priority: 10 },
        { mapName: 'users', name: 'merge-b', priority: 20, keyPattern: 'admin-*' },
      ],
    });
  });

  // =========================================================================
  // Domain 7: Client Events (9 messages)
  // =========================================================================

  test('SERVER_EVENT', () => {
    writeFixture('SERVER_EVENT', {
      type: 'SERVER_EVENT',
      payload: {
        mapName: 'users',
        eventType: 'PUT',
        key: 'user-1',
        record: {
          value: { name: 'Alice' },
          timestamp,
        },
      },
    });
  });

  test('SERVER_BATCH_EVENT', () => {
    writeFixture('SERVER_BATCH_EVENT', {
      type: 'SERVER_BATCH_EVENT',
      payload: {
        events: [
          {
            mapName: 'users',
            eventType: 'PUT',
            key: 'user-1',
            record: { value: 'v1', timestamp },
          },
          {
            mapName: 'users',
            eventType: 'REMOVE',
            key: 'user-2',
          },
        ],
      },
    });
  });

  test('GC_PRUNE', () => {
    writeFixture('GC_PRUNE', {
      type: 'GC_PRUNE',
      payload: {
        olderThan: timestamp,
      },
    });
  });

  test('AUTH_ACK', () => {
    writeFixture('AUTH_ACK', {
      type: 'AUTH_ACK',
      protocolVersion: 1,
    });
  });

  test('AUTH_FAIL', () => {
    writeFixture('AUTH_FAIL', {
      type: 'AUTH_FAIL',
      error: 'invalid token',
      code: 401,
    });
  });

  test('ERROR', () => {
    writeFixture('ERROR', {
      type: 'ERROR',
      payload: {
        code: 500,
        message: 'internal server error',
        details: 'stack trace here',
      },
    });
  });

  test('LOCK_GRANTED', () => {
    writeFixture('LOCK_GRANTED', {
      type: 'LOCK_GRANTED',
      payload: { requestId: 'lock-1', name: 'my-lock', fencingToken: 42 },
    });
  });

  test('LOCK_RELEASED', () => {
    writeFixture('LOCK_RELEASED', {
      type: 'LOCK_RELEASED',
      payload: { requestId: 'lock-1', name: 'my-lock', success: true },
    });
  });

  test('SYNC_RESET_REQUIRED', () => {
    writeFixture('SYNC_RESET_REQUIRED', {
      type: 'SYNC_RESET_REQUIRED',
      payload: {
        mapName: 'users',
        reason: 'partition ownership changed',
      },
    });
  });

  // =========================================================================
  // Domain 8: HTTP Sync (standalone -- not Message variants, separate test)
  // =========================================================================

  test('HTTP_SYNC_REQUEST (standalone)', () => {
    const data = {
      clientId: 'client-1',
      clientHlc: timestamp,
      operations: [
        {
          id: 'op-1',
          mapName: 'users',
          key: 'user-1',
          record: { value: 'Alice', timestamp },
        },
      ],
      syncMaps: [
        { mapName: 'users', lastSyncTimestamp: timestamp },
      ],
    };
    const packed = pack(data);
    fs.writeFileSync(
      path.join(FIXTURES_DIR, 'HTTP_SYNC_REQUEST.msgpack'),
      packed
    );
    fs.writeFileSync(
      path.join(FIXTURES_DIR, 'HTTP_SYNC_REQUEST.json'),
      JSON.stringify(data, null, 2)
    );
  });

  test('HTTP_SYNC_RESPONSE (standalone)', () => {
    const data = {
      serverHlc: timestamp,
      ack: { lastId: 'op-1' },
      deltas: [
        {
          mapName: 'users',
          records: [
            {
              key: 'user-1',
              record: { value: 'Alice', timestamp },
              eventType: 'PUT',
            },
          ],
          serverSyncTimestamp: timestamp,
        },
      ],
    };
    const packed = pack(data);
    fs.writeFileSync(
      path.join(FIXTURES_DIR, 'HTTP_SYNC_RESPONSE.msgpack'),
      packed
    );
    fs.writeFileSync(
      path.join(FIXTURES_DIR, 'HTTP_SYNC_RESPONSE.json'),
      JSON.stringify(data, null, 2)
    );
  });

  // =========================================================================
  // Validation: count total fixtures generated
  // =========================================================================

  test('generated at least 40 fixtures', () => {
    const files = fs.readdirSync(FIXTURES_DIR).filter(f => f.endsWith('.msgpack'));
    expect(files.length).toBeGreaterThanOrEqual(40);
  });
});
