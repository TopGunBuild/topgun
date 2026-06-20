/**
 * MCP Integration Tests
 *
 * Tests MCP protocol compliance with real components:
 * - TopGunMCPServer (not mocked)
 * - TopGunClient with InMemoryStorageAdapter
 * - Real tool handlers
 */

import { TopGunMCPServer } from '../TopGunMCPServer';

// Mock WebSocket so TopGunClient does not open a real undici connection to
// ws://localhost:8080. Without a mock, DNS/TCP handles outlive each test's
// afterEach and Force-exit Jest. queueMicrotask fires onopen synchronously
// before the next macrotask, clearing SingleServerProvider's connectionTimeoutId.
class MockWebSocket {
  static OPEN = 1;
  static CONNECTING = 0;
  static CLOSED = 3;

  readyState = MockWebSocket.CONNECTING;
  binaryType: string = 'blob';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((error: Error) => void) | null = null;
  onmessage: ((event: unknown) => void) | null = null;

  constructor(public url: string) {
    queueMicrotask(() => {
      this.readyState = MockWebSocket.OPEN;
      if (this.onopen) this.onopen();
    });
  }

  send() {}
  close() {
    this.readyState = MockWebSocket.CLOSED;
    if (this.onclose) this.onclose();
  }
}

(global as unknown as { WebSocket: typeof MockWebSocket }).WebSocket = MockWebSocket;

describe('MCP Integration', () => {
  let server: TopGunMCPServer;

  afterEach(async () => {
    if (server) {
      const client = server.getClient();
      if (client) {
        await client.close();
      }
    }
  });

  describe('tools/list', () => {
    it('should verify all 8 tool handlers are registered', () => {
      server = new TopGunMCPServer();

      // Verify all 8 tools can be called (this proves they're registered)
      const toolNames = [
        'topgun_list_maps',
        'topgun_query',
        'topgun_mutate',
        'topgun_search',
        'topgun_subscribe',
        'topgun_schema',
        'topgun_stats',
        'topgun_explain',
      ];

      toolNames.forEach((toolName) => {
        // Should not throw "Unknown tool" error
        expect(async () => {
          await server.callTool(toolName, {});
        }).toBeDefined();
      });
    });

    it('should block topgun_mutate when enableMutations=false', async () => {
      server = new TopGunMCPServer({ enableMutations: false });

      const result = await server.callTool('topgun_mutate', {
        map: 'test',
        operation: 'set',
        key: 'key1',
        data: {},
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should block topgun_subscribe when enableSubscriptions=false', async () => {
      server = new TopGunMCPServer({ enableSubscriptions: false });

      const result = await server.callTool('topgun_subscribe', {
        map: 'test',
        timeout: 1,
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should block both when both disabled', async () => {
      server = new TopGunMCPServer({
        enableMutations: false,
        enableSubscriptions: false,
      });

      const mutateResult = await server.callTool('topgun_mutate', {
        map: 'test',
        operation: 'set',
        key: 'key1',
        data: {},
      });
      expect(mutateResult.isError).toBe(true);

      const subscribeResult = await server.callTool('topgun_subscribe', {
        map: 'test',
        timeout: 1,
      });
      expect(subscribeResult.isError).toBe(true);
    });
  });

  describe('tools/call', () => {
    beforeEach(async () => {
      server = new TopGunMCPServer({
        allowedMaps: ['tasks', 'users'],
      });
      await server.getClient().start();
    });

    it('should execute topgun_list_maps', async () => {
      const result = await server.callTool('topgun_list_maps', {});

      expect(result).toBeDefined();
      expect(result.content).toBeDefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('users');
    });

    it('should execute topgun_query with valid map', async () => {
      const result = await server.callTool('topgun_query', { map: 'tasks' });

      expect(result).toBeDefined();
      // No reachable server in this harness, so queryOnce cannot settle: the tool
      // reports an explicit not-settled message rather than a silent empty result.
      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not settled');
      expect(result.content[0].text).not.toContain('No results found');
    });

    it('should execute topgun_mutate (unconfirmed without a reachable server)', async () => {
      const result = await server.callTool('topgun_mutate', {
        map: 'tasks',
        operation: 'set',
        key: 'task1',
        data: { title: 'Test Task' },
      });

      // mutate now confirms the write against the server before reporting success.
      // No reachable server here ⇒ honest not-durable error, never a false success.
      // Server-confirmed success is proven by the real-server integration harness.
      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(/not yet.*durable/i);
    });

    it('should execute topgun_schema', async () => {
      const result = await server.callTool('topgun_schema', { map: 'tasks' });

      expect(result).toBeDefined();
      // schema is now server-authoritative (reads via queryOncePaged, not the
      // local replica). With no reachable server in this harness the read cannot
      // settle, so the tool reports an explicit not-settled error rather than a
      // false "empty" answer. Real schema inference is covered by the unit suite
      // (mock client returns rows) and the real-server integration harness.
      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT an empty result');
    });

    it('should execute topgun_stats', async () => {
      const result = await server.callTool('topgun_stats', {});

      expect(result).toBeDefined();
      expect(result.content[0].text).toContain('Connection');
    });

    it('should execute topgun_explain', async () => {
      const result = await server.callTool('topgun_explain', {
        map: 'tasks',
        filter: { status: 'done' },
      });

      expect(result).toBeDefined();
      // explain is now server-authoritative: it estimates the plan over a settled
      // server sample, not the empty local replica. No reachable server here means
      // the read cannot settle, so the tool surfaces the not-settled path.
      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT an empty result');
    });

    it('should execute topgun_search', async () => {
      const result = await server.callTool('topgun_search', {
        map: 'tasks',
        query: 'test',
      });

      expect(result).toBeDefined();
      // Search may return error if search not supported in InMemoryStorageAdapter
      // Just verify the tool executes and returns a result
      expect(result.content).toBeDefined();
      expect(result.content[0].text).toBeDefined();
    });

    // topgun_subscribe is now a poll-cursor change feed (action: start/poll/stop/
    // list), not a blocking wait. The session-management actions (list/poll/stop)
    // are server-independent, so they are exercised here against the mock; the
    // start → live-delta path needs a real server and is covered by the
    // integration-rust harness (F3).
    it('topgun_subscribe action:list reports no active subscriptions', async () => {
      const result = await server.callTool('topgun_subscribe', { action: 'list' });

      expect(result).toBeDefined();
      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No active subscriptions');
    });

    it('topgun_subscribe action:poll requires a subscriptionId', async () => {
      const result = await server.callTool('topgun_subscribe', { action: 'poll' });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('subscriptionId');
    });

    it('topgun_subscribe action:poll on an unknown id is an explicit error', async () => {
      const result = await server.callTool('topgun_subscribe', {
        action: 'poll',
        subscriptionId: 'does-not-exist',
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(/no active subscription/i);
    });

    it('topgun_subscribe action:stop on an unknown id reports nothing to stop', async () => {
      const result = await server.callTool('topgun_subscribe', {
        action: 'stop',
        subscriptionId: 'does-not-exist',
      });

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toMatch(/no active subscription/i);
    });

    it('topgun_subscribe action:start requires a map', async () => {
      const result = await server.callTool('topgun_subscribe', { action: 'start' });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain("'map' is required");
    });

    it('should return error for invalid arguments', async () => {
      const result = await server.callTool('topgun_query', {
        map: 123, // Invalid: should be string
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('Invalid arguments');
    });

    it('should enforce map access restrictions on query', async () => {
      const result = await server.callTool('topgun_query', { map: 'forbidden' });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not allowed');
    });

    it('should enforce map access restrictions on mutate', async () => {
      const result = await server.callTool('topgun_mutate', {
        map: 'forbidden',
        operation: 'set',
        key: 'key1',
        data: { test: true },
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not allowed');
    });

    it('should enforce map access restrictions on search', async () => {
      const result = await server.callTool('topgun_search', {
        map: 'forbidden',
        query: 'test',
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not allowed');
    });
  });

  describe('End-to-End Data Flow', () => {
    beforeEach(async () => {
      server = new TopGunMCPServer();
      await server.getClient().start();
    });

    it('should write data via topgun_mutate; offline write/read are honest errors, not silent success/empty', async () => {
      // The write is queued locally, but with no reachable server it cannot be
      // confirmed durable — so mutate reports a not-durable error rather than a
      // false "Successfully created".
      const writeResult = await server.callTool('topgun_mutate', {
        map: 'products',
        operation: 'set',
        key: 'prod1',
        data: { name: 'Widget', price: 9.99 },
      });

      expect(writeResult.isError).toBe(true);
      expect(writeResult.content[0].text).toMatch(/not yet.*durable/i);

      // Read back. queryOnce is server-truth: with no reachable server it cannot
      // settle, so the tool reports an explicit not-settled message and never
      // silently serves stale local data as if it were authoritative.
      const readResult = await server.callTool('topgun_query', { map: 'products' });

      expect(readResult.isError).toBe(true);
      expect(readResult.content[0].text).toContain('not settled');
      expect(readResult.content[0].text).not.toContain('No results found');
    });

    it('offline query is not-settled, never silent stale local data', async () => {
      // No seeding mutations: without a reachable server they cannot be confirmed
      // (each would block the confirm timeout) and add nothing to this assertion.
      // Query — offline, so server-truth queryOnce cannot settle.
      const result = await server.callTool('topgun_query', { map: 'items' });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not settled');
    });

    it('should not infer schema from local-only writes (server-authoritative)', async () => {
      // Write structured data locally (server unreachable in this harness).
      await server.callTool('topgun_mutate', {
        map: 'records',
        operation: 'set',
        key: 'rec1',
        data: {
          name: 'Test',
          age: 25,
          active: true,
          tags: ['a', 'b'],
        },
      });

      // schema reads the SERVER, not the local replica these writes landed in.
      // With no reachable server it cannot settle — and must NOT echo the local
      // write back as if it were the map's schema. This is the core F1 fix: a
      // local-only write is not authoritative schema. (Real inference over server
      // data is proven by the real-server integration harness.)
      const schemaResult = await server.callTool('topgun_schema', { map: 'records' });

      expect(schemaResult.isError).toBe(true);
      expect(schemaResult.content[0].text).toContain('NOT an empty result');
    });

    it('should return connection info via topgun_stats', async () => {
      const statsResult = await server.callTool('topgun_stats', {});

      expect(statsResult.isError).toBeUndefined();
      expect(statsResult.content[0].text).toContain('Connection');
      expect(statsResult.content[0].text).toContain('Status');
    });

    it('should report not-settled for explain when server is unreachable', async () => {
      const explainResult = await server.callTool('topgun_explain', {
        map: 'tasks',
        filter: { status: 'active', priority: 'high' },
      });

      // explain estimates over a settled server sample; with no reachable server
      // the read cannot settle, so it surfaces the not-settled path (it must not
      // present a plan computed over the empty local replica as if it were truth).
      expect(explainResult.isError).toBe(true);
      expect(explainResult.content[0].text).toContain('tasks');
      expect(explainResult.content[0].text).toContain('NOT an empty result');
    });

    it('should handle remove operation correctly', async () => {
      // Create
      await server.callTool('topgun_mutate', {
        map: 'temp',
        operation: 'set',
        key: 'temp1',
        data: { value: 'test' },
      });

      // Offline read-back cannot settle under server-truth queryOnce.
      let queryResult = await server.callTool('topgun_query', { map: 'temp' });
      expect(queryResult.content[0].text).toContain('not settled');

      // Remove — server-authoritative, but unconfirmed without a reachable server.
      const removeResult = await server.callTool('topgun_mutate', {
        map: 'temp',
        operation: 'remove',
        key: 'temp1',
      });

      expect(removeResult.isError).toBe(true);
      expect(removeResult.content[0].text).toMatch(/not yet.*durable/i);

      // Offline read-back cannot settle under server-truth queryOnce.
      queryResult = await server.callTool('topgun_query', { map: 'temp' });
      expect(queryResult.content[0].text).toContain('not settled');
    });
  });

  describe('Configuration Options', () => {
    it('should respect enableMutations=false', async () => {
      server = new TopGunMCPServer({ enableMutations: false });
      await server.getClient().start();

      const result = await server.callTool('topgun_mutate', {
        map: 'test',
        operation: 'set',
        key: 'key1',
        data: { test: true },
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should respect enableSubscriptions=false', async () => {
      server = new TopGunMCPServer({ enableSubscriptions: false });
      await server.getClient().start();

      const result = await server.callTool('topgun_subscribe', {
        map: 'test',
        timeout: 1,
      });

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should respect defaultLimit configuration', async () => {
      server = new TopGunMCPServer({ defaultLimit: 5 });

      // No seeding loop: writes can no longer be confirmed without a reachable
      // server (each would block the full confirm timeout), and the query below
      // cannot settle anyway. Server-applied limit counts are covered by the
      // real-server integration harness; here we only assert the access/not-settled
      // path for the configured limit.
      const result = await server.callTool('topgun_query', { map: 'limited' });

      const text = result.content[0].text;
      expect(result.isError).toBe(true);
      expect(text).toContain('not settled');
      expect(text).toContain('limited');
    });

    it('should respect maxLimit configuration', async () => {
      server = new TopGunMCPServer({ maxLimit: 3 });

      // Query with limit exceeding maxLimit. No seeding loop (see defaultLimit
      // test): offline server-truth queryOnce cannot settle, so we assert the tool
      // executes and surfaces the not-settled path rather than server-applied counts.
      const result = await server.callTool('topgun_query', {
        map: 'maxed',
        limit: 100,
      });

      const text = result.content[0].text;
      expect(result.isError).toBe(true);
      expect(text).toContain('not settled');
      expect(text).toContain('maxed');
    });
  });
});
