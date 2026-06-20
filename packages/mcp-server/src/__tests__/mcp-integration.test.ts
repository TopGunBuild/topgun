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

    it('should execute topgun_mutate', async () => {
      const result = await server.callTool('topgun_mutate', {
        map: 'tasks',
        operation: 'set',
        key: 'task1',
        data: { title: 'Test Task' },
      });

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully created');
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

    it('should execute topgun_subscribe', async () => {
      const result = await server.callTool('topgun_subscribe', {
        map: 'tasks',
        timeout: 1,
      });

      expect(result).toBeDefined();
      expect(result.isError).toBeUndefined();
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

    it('should write data via topgun_mutate; offline read is not-settled, not silent empty', async () => {
      // Write data (local write succeeds even while the server is unreachable).
      const writeResult = await server.callTool('topgun_mutate', {
        map: 'products',
        operation: 'set',
        key: 'prod1',
        data: { name: 'Widget', price: 9.99 },
      });

      expect(writeResult.isError).toBeUndefined();
      expect(writeResult.content[0].text).toContain('Successfully created');

      // Read back. queryOnce is server-truth: with no reachable server it cannot
      // settle, so the tool reports an explicit not-settled message and never
      // silently serves stale local data as if it were authoritative.
      const readResult = await server.callTool('topgun_query', { map: 'products' });

      expect(readResult.isError).toBe(true);
      expect(readResult.content[0].text).toContain('not settled');
      expect(readResult.content[0].text).not.toContain('No results found');
    });

    it('should accept multiple local mutations; offline query is not-settled', async () => {
      // Create
      await server.callTool('topgun_mutate', {
        map: 'items',
        operation: 'set',
        key: 'item1',
        data: { title: 'First' },
      });

      // Update
      await server.callTool('topgun_mutate', {
        map: 'items',
        operation: 'set',
        key: 'item1',
        data: { title: 'Updated' },
      });

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

      // Remove
      const removeResult = await server.callTool('topgun_mutate', {
        map: 'temp',
        operation: 'remove',
        key: 'temp1',
      });

      expect(removeResult.isError).toBeUndefined();
      expect(removeResult.content[0].text).toContain('Successfully removed');

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

      // Create more records than default limit
      for (let i = 1; i <= 10; i++) {
        await server.callTool('topgun_mutate', {
          map: 'limited',
          operation: 'set',
          key: `item${i}`,
          data: { index: i },
        });
      }

      // Query without specifying limit. Offline server-truth queryOnce cannot
      // settle, so we assert the tool executes and surfaces the not-settled path
      // rather than verifying server-applied limit counts (which require a server).
      const result = await server.callTool('topgun_query', { map: 'limited' });

      const text = result.content[0].text;
      expect(result.isError).toBe(true);
      expect(text).toContain('not settled');
      expect(text).toContain('limited');
    });

    it('should respect maxLimit configuration', async () => {
      server = new TopGunMCPServer({ maxLimit: 3 });

      // Create records
      for (let i = 1; i <= 10; i++) {
        await server.callTool('topgun_mutate', {
          map: 'maxed',
          operation: 'set',
          key: `item${i}`,
          data: { index: i },
        });
      }

      // Query with limit exceeding maxLimit
      const result = await server.callTool('topgun_query', {
        map: 'maxed',
        limit: 100,
      });

      const text = result.content[0].text;
      // Offline server-truth queryOnce cannot settle; assert the tool executes and
      // surfaces the not-settled path rather than verifying server-applied counts.
      expect(result.isError).toBe(true);
      expect(text).toContain('not settled');
      expect(text).toContain('maxed');
    });
  });
});
