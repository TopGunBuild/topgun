/**
 * TopGunMCPServer Unit Tests
 */

import { TopGunMCPServer } from '../TopGunMCPServer';
import type { MCPServerConfig } from '../types';

// Mock @modelcontextprotocol/sdk
jest.mock('@modelcontextprotocol/sdk/server/index.js', () => ({
  Server: jest.fn().mockImplementation(() => ({
    setRequestHandler: jest.fn(),
    connect: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
  })),
}));

jest.mock('@modelcontextprotocol/sdk/server/stdio.js', () => ({
  StdioServerTransport: jest.fn().mockImplementation(() => ({})),
}));

jest.mock('@modelcontextprotocol/sdk/types.js', () => ({
  CallToolRequestSchema: Symbol('CallToolRequestSchema'),
  ListToolsRequestSchema: Symbol('ListToolsRequestSchema'),
}));

// Mock WebSocket for TopGunClient. Uses queueMicrotask (not setTimeout) so the
// onopen fires before the first awaited expression in each test, giving
// SingleServerProvider's onopen wrapper a chance to clear its connection-timeout
// before the test assertions run. queueMicrotask has no associated timer handle,
// so --detectOpenHandles does not report it as a leak.
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

// Mock crypto.randomUUID
let uuidCounter = 0;
Object.defineProperty(global, 'crypto', {
  value: {
    randomUUID: () => `test-uuid-${++uuidCounter}`,
  },
});

describe('TopGunMCPServer', () => {
  // Every TopGunMCPServer created inside an individual test is registered here so
  // afterEach can close the underlying TopGunClient. Without this, the client's
  // SingleServerProvider leaves its connection-timeout timer pending and Jest's
  // event loop never drains naturally.
  const servers: TopGunMCPServer[] = [];

  function makeServer(config?: MCPServerConfig): TopGunMCPServer {
    const s = new TopGunMCPServer(config);
    servers.push(s);
    return s;
  }

  beforeEach(() => {
    uuidCounter = 0;
  });

  afterEach(async () => {
    // Close each server's underlying TopGunClient so all async timers are cleared
    // before Jest moves to the next test. stop() is guarded by isStarted — since
    // these tests skip start(), close the client directly via getClient().close().
    for (const s of servers) {
      await s.getClient().close();
    }
    servers.length = 0;
  });

  describe('constructor', () => {
    it('should create server with default config', () => {
      const server = makeServer();
      const config = server.getConfig();

      expect(config.name).toBe('topgun-mcp-server');
      expect(config.version).toBe('1.0.0');
      expect(config.topgunUrl).toBe('ws://localhost:8080');
      expect(config.enableMutations).toBe(true);
      expect(config.enableSubscriptions).toBe(true);
      expect(config.defaultLimit).toBe(10);
      expect(config.maxLimit).toBe(100);
    });

    it('should accept custom config', () => {
      const config: MCPServerConfig = {
        name: 'custom-server',
        version: '2.0.0',
        topgunUrl: 'ws://custom:9000',
        allowedMaps: ['tasks', 'users'],
        enableMutations: false,
        defaultLimit: 20,
        debug: true,
      };

      const server = makeServer(config);
      const resolvedConfig = server.getConfig();

      expect(resolvedConfig.name).toBe('custom-server');
      expect(resolvedConfig.version).toBe('2.0.0');
      expect(resolvedConfig.topgunUrl).toBe('ws://custom:9000');
      expect(resolvedConfig.allowedMaps).toEqual(['tasks', 'users']);
      expect(resolvedConfig.enableMutations).toBe(false);
      expect(resolvedConfig.defaultLimit).toBe(20);
      expect(resolvedConfig.debug).toBe(true);
    });

    it('should set auth token when provided', () => {
      const server = makeServer({
        authToken: 'test-jwt-token',
      });

      expect(server.getConfig().authToken).toBe('test-jwt-token');
    });
  });

  describe('getClient', () => {
    it('should return TopGunClient instance', () => {
      const server = makeServer();
      const client = server.getClient();

      expect(client).toBeDefined();
      expect(typeof client.getMap).toBe('function');
    });
  });

  describe('getServer', () => {
    it('should return MCP Server instance', () => {
      const server = makeServer();
      const mcpServer = server.getServer();

      expect(mcpServer).toBeDefined();
      expect(typeof mcpServer.setRequestHandler).toBe('function');
    });
  });

  describe('callTool', () => {
    it('should execute query tool', async () => {
      const server = makeServer();

      const result = await server.callTool('topgun_query', { map: 'tasks' });

      expect(result).toBeDefined();
      // With no reachable server, queryOnce cannot settle: the tool must report
      // an explicit not-settled message, never conflate it with an empty result.
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'not settled',
      );
      expect((result as { content: Array<{ text: string }> }).content[0].text).not.toContain(
        'No results found',
      );
    });

    it('should execute list_maps tool', async () => {
      const server = makeServer({
        allowedMaps: ['tasks', 'users'],
      });

      const result = await server.callTool('topgun_list_maps', {});

      expect(result).toBeDefined();
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain('tasks');
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain('users');
    });

    it('should execute stats tool', async () => {
      const server = makeServer();

      const result = await server.callTool('topgun_stats', {});

      expect(result).toBeDefined();
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'Connection',
      );
    });

    it('should throw for unknown tool', async () => {
      const server = makeServer();

      await expect(server.callTool('unknown_tool', {})).rejects.toThrow('Unknown tool');
    });
  });

  describe('mutations', () => {
    it('should allow mutations when enabled (gated on server confirmation)', async () => {
      const server = makeServer({ enableMutations: true });

      const result = await server.callTool('topgun_mutate', {
        map: 'tasks',
        operation: 'set',
        key: 'task1',
        data: { title: 'Test' },
      });

      // Mutations are NOT blocked (no "disabled" error) — they reach the write
      // path. With no reachable server in this harness the write cannot be
      // confirmed durable, so the tool honestly reports a not-durable error rather
      // than a false "Successfully created". Server-confirmed success is proven by
      // the real-server integration harness.
      expect((result as { content: Array<{ text: string }> }).content[0].text).not.toContain(
        'disabled',
      );
      expect((result as { isError?: boolean }).isError).toBe(true);
      expect((result as { content: Array<{ text: string }> }).content[0].text).toMatch(
        /not yet.*durable/i,
      );
    });

    it('should block mutations when disabled', async () => {
      const server = makeServer({ enableMutations: false });

      const result = await server.callTool('topgun_mutate', {
        map: 'tasks',
        operation: 'set',
        key: 'task1',
        data: { title: 'Test' },
      });

      expect((result as { isError?: boolean }).isError).toBe(true);
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'disabled',
      );
    });
  });

  describe('map restrictions', () => {
    it('should allow access to allowed maps', async () => {
      const server = makeServer({
        allowedMaps: ['tasks'],
      });

      const result = await server.callTool('topgun_query', { map: 'tasks' });

      // Access is allowed (no "not allowed" error). Offline server-truth means the
      // query itself cannot settle, so it is a not-settled error rather than a
      // map-access denial — assert the access path, not the connectivity outcome.
      expect((result as { content: Array<{ text: string }> }).content[0].text).not.toContain(
        'not allowed',
      );
    });

    it('should deny access to restricted maps', async () => {
      const server = makeServer({
        allowedMaps: ['tasks'],
      });

      const result = await server.callTool('topgun_query', { map: 'users' });

      expect((result as { isError?: boolean }).isError).toBe(true);
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'not allowed',
      );
    });
  });
});
