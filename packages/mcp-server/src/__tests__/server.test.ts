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

// Mock WebSocket for TopGunClient
class MockWebSocket {
  static OPEN = 1;
  static CLOSED = 3;

  readyState = MockWebSocket.OPEN;
  binaryType: string = 'blob';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((error: Error) => void) | null = null;
  onmessage: ((event: unknown) => void) | null = null;

  constructor(public url: string) {
    setTimeout(() => {
      if (this.onopen) this.onopen();
    }, 0);
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
  beforeEach(() => {
    uuidCounter = 0;
  });

  describe('constructor', () => {
    it('should create server with default config', () => {
      const server = new TopGunMCPServer();
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

      const server = new TopGunMCPServer(config);
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
      const server = new TopGunMCPServer({
        authToken: 'test-jwt-token',
      });

      expect(server.getConfig().authToken).toBe('test-jwt-token');
    });
  });

  describe('getClient', () => {
    it('should return TopGunClient instance', () => {
      const server = new TopGunMCPServer();
      const client = server.getClient();

      expect(client).toBeDefined();
      expect(typeof client.getMap).toBe('function');
    });
  });

  describe('getServer', () => {
    it('should return MCP Server instance', () => {
      const server = new TopGunMCPServer();
      const mcpServer = server.getServer();

      expect(mcpServer).toBeDefined();
      expect(typeof mcpServer.setRequestHandler).toBe('function');
    });
  });

  describe('callTool', () => {
    it('should execute query tool', async () => {
      const server = new TopGunMCPServer();

      const result = await server.callTool('topgun_query', { map: 'tasks' });

      expect(result).toBeDefined();
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'No results found'
      );
    });

    it('should execute list_maps tool', async () => {
      const server = new TopGunMCPServer({
        allowedMaps: ['tasks', 'users'],
      });

      const result = await server.callTool('topgun_list_maps', {});

      expect(result).toBeDefined();
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain('tasks');
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain('users');
    });

    it('should execute stats tool', async () => {
      const server = new TopGunMCPServer();

      const result = await server.callTool('topgun_stats', {});

      expect(result).toBeDefined();
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'Connection'
      );
    });

    it('should throw for unknown tool', async () => {
      const server = new TopGunMCPServer();

      await expect(server.callTool('unknown_tool', {})).rejects.toThrow('Unknown tool');
    });
  });

  describe('mutations', () => {
    it('should allow mutations when enabled', async () => {
      const server = new TopGunMCPServer({ enableMutations: true });

      const result = await server.callTool('topgun_mutate', {
        map: 'tasks',
        operation: 'set',
        key: 'task1',
        data: { title: 'Test' },
      });

      expect((result as { isError?: boolean }).isError).toBeUndefined();
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'Successfully created'
      );
    });

    it('should block mutations when disabled', async () => {
      const server = new TopGunMCPServer({ enableMutations: false });

      const result = await server.callTool('topgun_mutate', {
        map: 'tasks',
        operation: 'set',
        key: 'task1',
        data: { title: 'Test' },
      });

      expect((result as { isError?: boolean }).isError).toBe(true);
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'disabled'
      );
    });
  });

  describe('map restrictions', () => {
    it('should allow access to allowed maps', async () => {
      const server = new TopGunMCPServer({
        allowedMaps: ['tasks'],
      });

      const result = await server.callTool('topgun_query', { map: 'tasks' });

      expect((result as { isError?: boolean }).isError).toBeUndefined();
    });

    it('should deny access to restricted maps', async () => {
      const server = new TopGunMCPServer({
        allowedMaps: ['tasks'],
      });

      const result = await server.callTool('topgun_query', { map: 'users' });

      expect((result as { isError?: boolean }).isError).toBe(true);
      expect((result as { content: Array<{ text: string }> }).content[0].text).toContain(
        'not allowed'
      );
    });
  });
});
