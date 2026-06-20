/**
 * MCP Tools Unit Tests
 */

import type { ToolContext, ResolvedMCPServerConfig } from '../types';
import { SubscriptionRegistry } from '../subscriptions';
import { QueryOnceUnsettledError } from '@topgunbuild/client';
import { handleQuery } from '../tools/query';
import { handleMutate } from '../tools/mutate';
import { handleSchema } from '../tools/schema';
import { handleStats } from '../tools/stats';
import { handleExplain } from '../tools/explain';
import { handleListMaps } from '../tools/listMaps';
import { handleSearch } from '../tools/search';

// Mock TopGunClient
class MockLWWMap {
  private data = new Map<string, unknown>();

  set(key: string, value: unknown) {
    this.data.set(key, value);
    return { value, timestamp: Date.now() };
  }

  get(key: string) {
    return this.data.get(key);
  }

  remove(key: string) {
    this.data.delete(key);
    return { value: null, timestamp: Date.now() };
  }

  entries() {
    return this.data.entries();
  }
}

interface MockSearchHit {
  key: string;
  value: null;
  score: number;
  matchedTerms: string[];
}

interface MockHybridSearchHit {
  key: string;
  score: number;
  methodScores: Partial<Record<'exact' | 'fullText' | 'semantic', number>>;
  value?: unknown;
}

class MockTopGunClient {
  private maps = new Map<string, MockLWWMap>();
  // Configurable BM25 search results (legacy — not used by handleSearch after this spec).
  searchResults: MockSearchHit[] = [];
  // Configurable hybrid search results + recorded options for assertion.
  hybridSearchResults: MockHybridSearchHit[] = [];
  lastHybridSearchOptions: unknown = null;
  // When set, hybridSearch rejects with this error to simulate server-side failures
  // (no embedding model, FTS not enabled for the map, etc.).
  hybridSearchRejection: Error | null = null;
  // When set, queryOncePaged rejects with this error to simulate offline / not-settled.
  queryOnceRejection: Error | null = null;
  // Override hasMore/cursor returned by queryOncePaged for pagination tests.
  queryOncePagedHasMore = false;
  queryOncePagedCursor: string | undefined = undefined;
  // Outcome confirmWrite resolves with — defaults to a server-confirmed write so
  // the happy path needs no setup; override to exercise offline/timeout/failed.
  confirmWriteOutcome: 'synced' | 'offline' | 'timeout' | 'failed' = 'synced';
  // Records (map, key) pairs confirmWrite was asked to confirm, for assertions.
  confirmWriteCalls: Array<{ map: string; key: string }> = [];

  getMap(name: string): MockLWWMap {
    if (!this.maps.has(name)) {
      this.maps.set(name, new MockLWWMap());
    }
    return this.maps.get(name)!;
  }

  async confirmWrite(
    map: string,
    key: string,
    _timeoutMs?: number,
  ): Promise<'synced' | 'offline' | 'timeout' | 'failed'> {
    this.confirmWriteCalls.push({ map, key });
    return this.confirmWriteOutcome;
  }

  getConnectionState() {
    return 'CONNECTED';
  }

  getServerUrl() {
    return 'ws://localhost:8080/ws';
  }

  async getAuthToken() {
    return 'mock-token';
  }

  isCluster() {
    return false;
  }

  getPendingOpsCount() {
    return 0;
  }

  isBackpressurePaused() {
    return false;
  }

  getConnectedNodes() {
    return [];
  }

  getPartitionMapVersion() {
    return 0;
  }

  isRoutingActive() {
    return false;
  }

  async search(_map: string, _query: string, _options?: unknown) {
    return this.searchResults;
  }

  async hybridSearch(_map: string, _query: string, options?: unknown) {
    // Record the options so tests can assert which methods/k were forwarded.
    this.lastHybridSearchOptions = options;
    if (this.hybridSearchRejection) {
      throw this.hybridSearchRejection;
    }
    return this.hybridSearchResults;
  }

  // One-shot paged read mirroring TopGunClient.queryOncePaged: resolves with
  // settled, authoritative server data including cursor metadata, or rejects
  // to simulate offline / not-settled.
  async queryOncePaged(
    mapName: string,
    filter: {
      where?: Record<string, unknown>;
      predicate?: { op: string; attribute: string; value: unknown };
      limit?: number;
      cursor?: string;
    } = {},
  ): Promise<{
    items: Array<Record<string, unknown> & { _key: string }>;
    cursor?: string;
    hasMore: boolean;
  }> {
    if (this.queryOnceRejection) {
      throw this.queryOnceRejection;
    }

    const lwwMap = this.maps.get(mapName);
    let items: Array<Record<string, unknown> & { _key: string }> = [];

    if (lwwMap) {
      for (const [key, value] of lwwMap.entries()) {
        if (value !== null && value !== undefined) {
          items.push({ ...(value as Record<string, unknown>), _key: key });
        }
      }
    }

    // Apply where filter
    if (filter.where) {
      const where = filter.where as Record<string, unknown>;
      items = items.filter((item) => Object.entries(where).every(([k, v]) => item[k] === v));
    }

    // Mirror the server's `_key IN (...)` predicate so key-targeted hydration
    // returns exactly the requested records (the real server injects `_key` as a
    // filterable column on every row).
    if (filter.predicate?.op === 'in' && filter.predicate.attribute === '_key') {
      const allowed = new Set(filter.predicate.value as string[]);
      items = items.filter((item) => allowed.has(item._key));
    }

    // Apply limit
    if (filter.limit !== undefined && filter.limit < items.length) {
      items = items.slice(0, filter.limit);
    }

    return {
      items,
      cursor: this.queryOncePagedCursor,
      hasMore: this.queryOncePagedHasMore,
    };
  }

  // Legacy queryOnce for any remaining callers outside handleQuery
  async queryOnce(
    mapName: string,
    filter: { where?: Record<string, unknown>; limit?: number } = {},
  ): Promise<Array<Record<string, unknown> & { _key: string }>> {
    const result = await this.queryOncePaged(mapName, filter);
    return result.items;
  }
}

function createTestContext(config?: Partial<ResolvedMCPServerConfig>): ToolContext {
  const client = new MockTopGunClient() as unknown as ToolContext['client'];
  return {
    client,
    subscriptions: new SubscriptionRegistry(client),
    config: {
      name: 'topgun-server',
      version: '1.0.0',
      topgunUrl: 'ws://localhost:8080',
      enableMutations: true,
      enableSubscriptions: true,
      defaultLimit: 10,
      maxLimit: 100,
      subscriptionTimeoutSeconds: 60,
      debug: false,
      ...config,
    },
  };
}

describe('MCP Tools', () => {
  describe('handleListMaps', () => {
    const realFetch = global.fetch;
    afterEach(() => {
      global.fetch = realFetch;
    });

    it('returns the configured allow-list without a server round-trip', async () => {
      const ctx = createTestContext({
        allowedMaps: ['tasks', 'users', 'products'],
      });
      const fetchSpy = jest.fn();
      global.fetch = fetchSpy as unknown as typeof fetch;

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('users');
      expect(result.content[0].text).toContain('products');
      // The allow-list IS the authoritative scope — no server enumeration needed.
      expect(fetchSpy).not.toHaveBeenCalled();
    });

    it('returns the real server catalog from the admin endpoint, never fabricated names', async () => {
      const ctx = createTestContext();
      const fetchSpy = jest.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          maps: [
            { name: 'orders', entryCount: 3 },
            { name: 'customers', entryCount: 1 },
          ],
        }),
      });
      global.fetch = fetchSpy as unknown as typeof fetch;

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBeUndefined();
      const text = result.content[0].text ?? '';
      // Hits the derived HTTP control-plane URL with the client's bearer token.
      expect(fetchSpy).toHaveBeenCalledWith(
        'http://localhost:8080/api/admin/maps',
        expect.objectContaining({
          headers: { Authorization: 'Bearer mock-token' },
        }),
      );
      expect(text).toContain('orders');
      expect(text).toContain('3 entries');
      expect(text).toContain('customers');
      expect(text).toContain('1 entry');
      // Negative control: the retired fabricated "common patterns" must be gone.
      expect(text).not.toMatch(/common (map )?pattern/i);
      expect(text).not.toContain('Blog posts');
    });

    it('reports DISCONNECTED honestly and does not guess', async () => {
      const ctx = createTestContext();
      (ctx.client as unknown as { getConnectionState: () => string }).getConnectionState = () =>
        'DISCONNECTED';
      const fetchSpy = jest.fn();
      global.fetch = fetchSpy as unknown as typeof fetch;

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(/not connected/i);
      expect(result.content[0].text).toMatch(/NOT an empty/i);
      // Never fabricates and never round-trips while offline.
      expect(fetchSpy).not.toHaveBeenCalled();
    });

    it('reports an explicit error when the token lacks admin access, not example names', async () => {
      const ctx = createTestContext();
      global.fetch = jest.fn().mockResolvedValue({
        ok: false,
        status: 403,
        text: async () => 'forbidden',
      }) as unknown as typeof fetch;

      const result = await handleListMaps({}, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(/admin-scoped token/i);
      expect(result.content[0].text).not.toContain('Blog posts');
    });
  });

  describe('handleQuery', () => {
    it('should query an empty map', async () => {
      const ctx = createTestContext();

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });

    it('should return results from populated map', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Test Task', status: 'todo' });
      map.set('task2', { title: 'Another Task', status: 'done' });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('2 result');
      expect(result.content[0].text).toContain('Test Task');
      expect(result.content[0].text).toContain('Another Task');
    });

    it('should filter results', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Test Task', status: 'todo' });
      map.set('task2', { title: 'Done Task', status: 'done' });

      const result = await handleQuery({ map: 'tasks', filter: { status: 'done' } }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).toContain('Done Task');
      expect(result.content[0].text).not.toContain('Test Task');
    });

    it('should respect limit', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 20; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, index: i });
      }

      const result = await handleQuery({ map: 'tasks', limit: 5 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('5 result');
    });

    it('should signal truncation and provide continuation cursor when hasMore is true', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      const map = mockClient.getMap('tasks');
      for (let i = 0; i < 5; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, index: i });
      }
      // Server-authoritative pagination signal: more results exist
      mockClient.queryOncePagedHasMore = true;
      mockClient.queryOncePagedCursor = 'next-page-cursor-abc';

      const result = await handleQuery({ map: 'tasks', limit: 5 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('5 result');
      // The agent must be told the view was capped — never a silent truncation.
      expect(result.content[0].text).toContain('More rows match than were returned');
      // The real continuation cursor must be surfaced so the agent can page forward.
      expect(result.content[0].text).toContain('cursor: "next-page-cursor-abc"');
    });

    it('should NOT signal truncation when results fit within the limit', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Only Task', status: 'todo' });

      const result = await handleQuery({ map: 'tasks', limit: 10 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).not.toContain('More rows match');
    });

    it('should thread cursor arg through to queryOncePaged when provided', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      const map = mockClient.getMap('tasks');
      map.set('task1', { title: 'Only Task', status: 'todo' });

      // Cursor is a valid schema field — the call must succeed and return normally.
      const result = await handleQuery({ map: 'tasks', cursor: 'page-2-cursor' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
    });

    it('should return continuation cursor in result text when hasMore is true', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('tasks').set('task1', { title: 'Task 1' });
      mockClient.queryOncePagedHasMore = true;
      mockClient.queryOncePagedCursor = 'cursor-token-xyz';

      const result = await handleQuery({ map: 'tasks', limit: 1 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('cursor: "cursor-token-xyz"');
      // Should NOT say there is no cursor to page through
      expect(result.content[0].text).not.toContain('no cursor to page through');
    });

    it('should NOT include continuation note when hasMore is false', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('tasks').set('task1', { title: 'Only Task', status: 'todo' });
      // Default: queryOncePagedHasMore = false

      const result = await handleQuery({ map: 'tasks', limit: 10 }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).not.toContain('More rows match');
      expect(result.content[0].text).not.toContain('cursor:');
    });

    it('should NOT render cursor:"undefined" when hasMore is true but no token is available', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      for (let i = 0; i < 5; i++) {
        mockClient.getMap('tasks').set(`task${i}`, { title: `Task ${i}`, index: i });
      }
      // Server signals more rows exist but returns NO usable cursor (the F7 gap).
      mockClient.queryOncePagedHasMore = true;
      mockClient.queryOncePagedCursor = undefined;

      const result = await handleQuery({ map: 'tasks', limit: 5 }, ctx);

      expect(result.isError).toBeUndefined();
      // The truncation is still disclosed...
      expect(result.content[0].text).toContain('More rows match than were returned');
      // ...but the agent is NEVER told to page with the string "undefined".
      expect(result.content[0].text).not.toContain('cursor: "undefined"');
      expect(result.content[0].text).not.toContain('cursor:');
      // Instead it is advised to narrow the query.
      expect(result.content[0].text).toContain('narrow with');
    });

    it('should deny access to restricted maps', async () => {
      const ctx = createTestContext({
        allowedMaps: ['users'],
      });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not allowed');
    });

    it('should return settled server data (not a silent empty)', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      // A record that exists authoritatively on the server.
      map.set('task1', { title: 'Server Record', status: 'todo' });

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('1 result');
      expect(result.content[0].text).toContain('Server Record');
      // Settled server data must never read as the offline/not-settled message.
      expect(result.content[0].text).not.toContain('not settled');
    });

    it('should surface an explicit not-settled message when offline', async () => {
      const ctx = createTestContext();
      (ctx.client as unknown as MockTopGunClient).queryOnceRejection = new QueryOnceUnsettledError(
        'offline',
        'tasks',
      );

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      // Must explicitly signal offline / not-settled, never the silent empty text.
      expect(result.content[0].text).toContain('not settled');
      expect(result.content[0].text).toContain('offline');
      expect(result.content[0].text).not.toContain('No results found');
    });

    it('should surface an explicit not-settled message on timeout', async () => {
      const ctx = createTestContext();
      (ctx.client as unknown as MockTopGunClient).queryOnceRejection = new QueryOnceUnsettledError(
        'timeout',
        'tasks',
      );

      const result = await handleQuery({ map: 'tasks' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('not settled');
      expect(result.content[0].text).toContain('timed out');
      expect(result.content[0].text).not.toContain('No results found');
    });

    it('should still report a settled-but-empty server result as no results', async () => {
      const ctx = createTestContext();

      const result = await handleQuery({ map: 'tasks' }, ctx);

      // Genuinely empty (but settled) server answer — distinct from offline.
      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });
  });

  describe('handleMutate', () => {
    it('should save a record only after the server confirms it', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'New Task', status: 'todo' },
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      // Upsert wording — never claims create-vs-update (cold cache can't tell).
      expect(result.content[0].text).toContain('Successfully saved');
      expect(result.content[0].text).toContain('confirmed on server');
      expect(result.content[0].text).toContain('task1');
      // The success was gated on a server confirmation for this exact (map, key).
      expect(mockClient.confirmWriteCalls).toEqual([{ map: 'tasks', key: 'task1' }]);
    });

    it('should use upsert wording for an existing record too', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Old Title', status: 'todo' });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Updated Title', status: 'done' },
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully saved');
    });

    it('should NOT report success when a set is not confirmed (offline)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.confirmWriteOutcome = 'offline';

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Unsynced', status: 'todo' },
        },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT yet durable');
      expect(result.content[0].text).not.toContain('Successfully');
    });

    it('should report an error when a set times out without server confirmation', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.confirmWriteOutcome = 'timeout';

      const result = await handleMutate(
        { map: 'tasks', operation: 'set', key: 'task1', data: { title: 'Slow' } },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT yet confirmed durable');
    });

    it('should remove a record once the server confirms it', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'To Delete', status: 'todo' });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'remove',
          key: 'task1',
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully removed');
      expect(result.content[0].text).toContain('confirmed on server');
    });

    it('should issue a server-authoritative remove for a key absent from the local cache', async () => {
      // F5: the record exists on the server but is NOT in the cold MCP cache. The
      // remove must still be issued (server-authoritative) and reported as done —
      // never silently no-op'd with a false "does not exist".
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      const removeSpy = jest.spyOn(mockClient.getMap('tasks'), 'remove');

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'remove',
          key: 'server-only-key',
        },
        ctx,
      );

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Successfully removed');
      expect(result.content[0].text).not.toContain('does not exist');
      // The tombstone was actually written (not short-circuited on cold cache)...
      expect(removeSpy).toHaveBeenCalledWith('server-only-key');
      // ...and success was gated on a server confirmation.
      expect(mockClient.confirmWriteCalls).toEqual([{ map: 'tasks', key: 'server-only-key' }]);
    });

    it('should NOT report a remove as done when it is not confirmed (offline)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.confirmWriteOutcome = 'offline';

      const result = await handleMutate({ map: 'tasks', operation: 'remove', key: 'task1' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('NOT yet durable');
      expect(result.content[0].text).not.toContain('Successfully');
    });

    it('should error when mutations are disabled', async () => {
      const ctx = createTestContext({ enableMutations: false });

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
          data: { title: 'Test' },
        },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('disabled');
    });

    it('should require data for set operation', async () => {
      const ctx = createTestContext();

      const result = await handleMutate(
        {
          map: 'tasks',
          operation: 'set',
          key: 'task1',
        },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('"data" is required');
    });
  });

  describe('handleSchema', () => {
    it('should report empty map', async () => {
      const ctx = createTestContext();

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('empty');
    });

    it('should infer schema from data', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', {
        title: 'Test',
        count: 42,
        active: true,
        createdAt: '2025-01-01T00:00:00Z',
      });

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('title: string');
      expect(result.content[0].text).toContain('count: number');
      expect(result.content[0].text).toContain('active: boolean');
      expect(result.content[0].text).toContain('timestamp');
    });

    it('should detect enum fields', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { status: 'todo' });
      map.set('task2', { status: 'in-progress' });
      map.set('task3', { status: 'done' });

      const result = await handleSchema({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('enum');
      expect(result.content[0].text).toContain('todo');
      expect(result.content[0].text).toContain('done');
    });
  });

  describe('handleStats', () => {
    it('should return connection stats', async () => {
      const ctx = createTestContext();

      const result = await handleStats({}, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('CONNECTED');
      expect(result.content[0].text).toContain('Pending Operations: 0');
    });

    it('should return map stats when specified', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Task 1' });
      map.set('task2', { title: 'Task 2' });

      const result = await handleStats({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('tasks');
      expect(result.content[0].text).toContain('Records: 2');
    });
  });

  describe('handleExplain', () => {
    it('should explain full scan without filter', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      map.set('task1', { title: 'Task 1' });
      map.set('task2', { title: 'Task 2' });

      const result = await handleExplain({ map: 'tasks' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('FULL_SCAN');
      expect(result.content[0].text).toContain('Total Records: 2');
    });

    it('should explain filter scan', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 10; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, status: i % 2 === 0 ? 'done' : 'todo' });
      }

      const result = await handleExplain({ map: 'tasks', filter: { status: 'done' } }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('FILTER_SCAN');
      expect(result.content[0].text).toContain('Apply filter');
      expect(result.content[0].text).toContain('Estimated Results: 5');
      expect(result.content[0].text).toContain('Selectivity: 50');
    });

    it('should provide recommendations for large datasets', async () => {
      const ctx = createTestContext();
      const map = (ctx.client as unknown as MockTopGunClient).getMap('tasks');
      for (let i = 0; i < 1000; i++) {
        map.set(`task${i}`, { title: `Task ${i}`, status: 'todo' });
      }

      const result = await handleExplain({ map: 'tasks', filter: { status: 'todo' } }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Recommendations');
    });
  });

  describe('handleSearch', () => {
    it('should return hydrated record body for a matched hit', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // Pre-populate the map so the local read-by-key finds the body.
      mockClient.getMap('tasks').set('task1', { title: 'Test Task', status: 'todo' });
      // Wire hybridSearch() to return a hit with the hybrid result shape (methodScores, no matchedTerms).
      mockClient.hybridSearchResults = [
        { key: 'task1', score: 0.42, methodScores: { fullText: 0.42 } },
      ];

      const result = await handleSearch({ map: 'tasks', query: 'test' }, ctx);

      expect(result.isError).toBeUndefined();
      // Body fields must be present in the output.
      expect(result.content[0].text).toContain('Test Task');
      expect(result.content[0].text).toContain('todo');
      // The old null placeholder must not appear.
      expect(result.content[0].text).not.toContain('Data: null');
    });

    it('should preserve score with fixed-precision rendering and per-method score breakdown', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('tasks').set('task1', { title: 'Test Task', status: 'todo' });
      mockClient.getMap('tasks').set('task2', { title: 'Smoke Task', status: 'done' });
      // Two hits: one plain three-decimal score, one rounding-sensitive score matching
      // the real smoke output (0.2876 → "0.288") to guard toFixed(3) against digit-dropping.
      mockClient.hybridSearchResults = [
        { key: 'task1', score: 0.42, methodScores: { fullText: 0.42 } },
        { key: 'task2', score: 0.2876, methodScores: { fullText: 0.2876 } },
      ];

      const result = await handleSearch({ map: 'tasks', query: 'test smoke' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('[Score: 0.420]');
      // Rounding-sensitive assertion: 0.2876 must render as 0.288, not 0.287.
      expect(result.content[0].text).toContain('[Score: 0.288]');
      // Per-method scores must appear in the output — the old matchedTerms line is gone.
      expect(result.content[0].text).toContain('Method scores:');
      expect(result.content[0].text).not.toContain('Matched:');
    });

    it('should emit the not-available marker when the hit key is absent from the server read', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // Intentionally do NOT pre-populate the map — the server-authoritative
      // hydration read (queryOncePaged) returns no body for this key.
      mockClient.hybridSearchResults = [
        { key: 'missing-key', score: 0.9, methodScores: { fullText: 0.9 } },
      ];

      // Handler must not throw; the hit must still appear with the pinned fallback marker.
      const result = await handleSearch({ map: 'tasks', query: 'term' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Data: (record body not available on the server)');
      // The old cold-cache marker must be gone — hydration is now server-authoritative.
      expect(result.content[0].text).not.toContain('not available locally');
    });

    it('should hydrate bodies from the server read, not the local replica', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // Body lives only in the store the server read (queryOncePaged) sees.
      mockClient.getMap('tasks').set('task1', { title: 'Server Body', status: 'open' });
      mockClient.hybridSearchResults = [
        { key: 'task1', score: 0.5, methodScores: { fullText: 0.5 } },
      ];

      const result = await handleSearch({ map: 'tasks', query: 'server' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Server Body');
      expect(result.content[0].text).toContain('open');
      expect(result.content[0].text).not.toContain('not available');
    });

    it('should hydrate a hit whose key sorts beyond the first page (key-targeted, not first-page scan)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // Populate well past the maxLimit page size; the single hit is the LAST key,
      // so a first-page scan (limit=maxLimit) ordered by insertion would miss it.
      for (let i = 0; i < 150; i++) {
        mockClient.getMap('big').set(`task${i}`, { title: `Task ${i}`, n: i });
      }
      mockClient.hybridSearchResults = [
        { key: 'task149', score: 0.99, methodScores: { fullText: 0.99 } },
      ];

      const result = await handleSearch({ map: 'big', query: 'task' }, ctx);

      expect(result.isError).toBeUndefined();
      // The body is hydrated by exact key, never marked "not available".
      expect(result.content[0].text).toContain('Task 149');
      expect(result.content[0].text).not.toContain('not available');
    });

    it('should still render ranked keys when the server hydration read does not settle', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchResults = [
        { key: 'task1', score: 0.7, methodScores: { fullText: 0.7 } },
      ];
      // hybridSearch succeeds (ranking), but the body hydration read is offline.
      mockClient.queryOnceRejection = new QueryOnceUnsettledError('offline', 'tasks');

      const result = await handleSearch({ map: 'tasks', query: 'term' }, ctx);

      // The hit is still surfaced (key + score), with the body honestly marked unfetched.
      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('task1');
      expect(result.content[0].text).toContain('did not settle');
    });

    it('should return the empty-results message when search returns no hits', async () => {
      const ctx = createTestContext();
      // hybridSearchResults defaults to [] — no configuration needed.

      const result = await handleSearch({ map: 'tasks', query: 'nothing' }, ctx);

      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('No results found');
    });

    it('should call hybridSearch (not the BM25 search path) with the default fullText method', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchResults = [
        { key: 'doc1', score: 0.75, methodScores: { fullText: 0.75 } },
      ];

      // Invoke with no methods arg — the default ['fullText'] must be forwarded.
      await handleSearch({ map: 'docs', query: 'auth' }, ctx);

      // The tool must have gone through hybridSearch, not the old BM25 search path.
      // If hybridSearch was called, lastHybridSearchOptions is populated; if search()
      // was called instead, it would remain null.
      const opts = mockClient.lastHybridSearchOptions as {
        methods: string[];
        k: number;
        minScore: number;
      };
      expect(opts).not.toBeNull();
      expect(opts.methods).toEqual(['fullText']);
    });

    it('should forward custom methods to hybridSearch and render multi-method score breakdown', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchResults = [
        { key: 'doc1', score: 0.82, methodScores: { exact: 0.6, fullText: 0.5 } },
      ];

      const result = await handleSearch(
        { map: 'docs', query: 'login', methods: ['exact', 'fullText'] },
        ctx,
      );

      const opts = mockClient.lastHybridSearchOptions as {
        methods: string[];
        k: number;
        minScore: number;
      };
      // Both requested methods must be forwarded verbatim.
      expect(opts.methods).toEqual(['exact', 'fullText']);
      // Output must render per-method scores for both legs so the agent can see the breakdown.
      expect(result.content[0].text).toContain('exact:');
      expect(result.content[0].text).toContain('fullText:');
    });

    it('should reject a semantic-only request honestly (vector search is dark on the server)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // If the gate failed and the request reached hybridSearch, lastHybridSearchOptions
      // would be populated — assert it stays null to prove the leg never ran.
      const result = await handleSearch(
        { map: 'docs', query: 'login', methods: ['semantic'] },
        ctx,
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain('Semantic (vector) search is not yet available');
      // The message must tell the agent how to retry without the semantic leg.
      expect(result.content[0].text).toContain('methods: ["fullText"]');
      // The dead leg was never invoked — no silent Noop round-trip.
      expect(mockClient.lastHybridSearchOptions).toBeNull();
    });

    it('should skip the semantic leg with a note when combined with a working method', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.getMap('docs').set('doc1', { title: 'Login Guide' });
      mockClient.hybridSearchResults = [
        { key: 'doc1', score: 0.6, methodScores: { fullText: 0.6 } },
      ];

      const result = await handleSearch(
        { map: 'docs', query: 'login', methods: ['fullText', 'semantic'] },
        ctx,
      );

      // The working leg still returns results...
      expect(result.isError).toBeUndefined();
      expect(result.content[0].text).toContain('Login Guide');
      // ...and the agent is told the semantic leg was skipped — never silently dropped.
      expect(result.content[0].text).toContain('"semantic" method was skipped');
      // Only the non-semantic methods were forwarded to the server.
      const opts = mockClient.lastHybridSearchOptions as { methods: string[] };
      expect(opts.methods).toEqual(['fullText']);
    });

    it('should point the agent at topgun_query when full-text search is not enabled for the map', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      mockClient.hybridSearchRejection = new Error("FTS is not enabled for map 'docs'");

      const result = await handleSearch({ map: 'docs', query: 'login' }, ctx);

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toContain("Full-text search is not available for map 'docs'");
      expect(result.content[0].text).toContain('topgun_query');
    });

    it('should map the raw "index registry not found" server error to a friendly message (no raw leak)', async () => {
      const ctx = createTestContext();
      const mockClient = ctx.client as unknown as MockTopGunClient;
      // The exact low-level string the server emits for an unindexed auto-created map.
      mockClient.hybridSearchRejection = new Error('index registry not found for map');

      const result = await handleSearch({ map: 'fresh', query: 'anything' }, ctx);

      expect(result.isError).toBe(true);
      // Friendly, actionable text — not the raw internal error.
      expect(result.content[0].text).toContain("Full-text search is not available for map 'fresh'");
      expect(result.content[0].text).toContain('topgun_query');
      expect(result.content[0].text).not.toContain('index registry not found');
    });
  });
});
