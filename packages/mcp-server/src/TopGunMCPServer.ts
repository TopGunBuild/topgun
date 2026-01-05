/**
 * TopGun MCP Server
 *
 * Model Context Protocol server for TopGun database.
 * Enables AI assistants (Claude, Cursor) to interact with TopGun data.
 */

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { TopGunClient } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import type { MCPServerConfig, ResolvedMCPServerConfig, ToolContext } from './types';
import { allTools, toolHandlers } from './tools';

/**
 * Default configuration values
 */
const DEFAULT_CONFIG: ResolvedMCPServerConfig = {
  name: 'topgun-mcp-server',
  version: '1.0.0',
  topgunUrl: 'ws://localhost:8080',
  enableMutations: true,
  enableSubscriptions: true,
  defaultLimit: 10,
  maxLimit: 100,
  subscriptionTimeoutSeconds: 60,
  debug: false,
};

/**
 * TopGun MCP Server
 *
 * Provides MCP protocol interface for AI assistants to interact
 * with TopGun databases.
 *
 * @example
 * ```typescript
 * const server = new TopGunMCPServer({
 *   topgunUrl: 'ws://localhost:8080',
 *   allowedMaps: ['tasks', 'users'],
 * });
 *
 * await server.start();
 * ```
 */
export class TopGunMCPServer {
  private readonly server: Server;
  private readonly client: TopGunClient;
  private readonly config: ResolvedMCPServerConfig;
  private readonly toolContext: ToolContext;
  private isStarted = false;
  private externalClient = false;

  constructor(config: MCPServerConfig = {}) {
    // Resolve configuration with defaults
    this.config = {
      name: config.name ?? DEFAULT_CONFIG.name,
      version: config.version ?? DEFAULT_CONFIG.version,
      topgunUrl: config.topgunUrl ?? DEFAULT_CONFIG.topgunUrl,
      authToken: config.authToken,
      allowedMaps: config.allowedMaps,
      enableMutations: config.enableMutations ?? DEFAULT_CONFIG.enableMutations,
      enableSubscriptions: config.enableSubscriptions ?? DEFAULT_CONFIG.enableSubscriptions,
      defaultLimit: config.defaultLimit ?? DEFAULT_CONFIG.defaultLimit,
      maxLimit: config.maxLimit ?? DEFAULT_CONFIG.maxLimit,
      subscriptionTimeoutSeconds:
        config.subscriptionTimeoutSeconds ?? DEFAULT_CONFIG.subscriptionTimeoutSeconds,
      debug: config.debug ?? DEFAULT_CONFIG.debug,
    };

    // Use provided client or create new one
    if (config.client) {
      this.client = config.client;
      this.externalClient = true;
    } else {
      // Create in-memory storage adapter for MCP server
      // Note: In production, you might want to use actual IDBAdapter or custom storage
      this.client = new TopGunClient({
        serverUrl: this.config.topgunUrl,
        storage: new InMemoryStorageAdapter(),
      });

      if (this.config.authToken) {
        this.client.setAuthToken(this.config.authToken);
      }
    }

    // Create tool context
    this.toolContext = {
      client: this.client,
      config: this.config,
    };

    // Initialize MCP server
    this.server = new Server(
      {
        name: this.config.name,
        version: this.config.version,
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    // Register handlers
    this.registerHandlers();

    this.log('TopGunMCPServer initialized', {
      topgunUrl: this.config.topgunUrl,
      allowedMaps: this.config.allowedMaps,
      enableMutations: this.config.enableMutations,
    });
  }

  /**
   * Register MCP protocol handlers
   */
  private registerHandlers(): void {
    // List available tools
    this.server.setRequestHandler(ListToolsRequestSchema, async () => {
      // Filter tools based on config
      let availableTools = [...allTools];

      if (!this.config.enableMutations) {
        availableTools = availableTools.filter((t) => t.name !== 'topgun_mutate');
      }

      if (!this.config.enableSubscriptions) {
        availableTools = availableTools.filter((t) => t.name !== 'topgun_subscribe');
      }

      this.log('tools/list called', { count: availableTools.length });

      return {
        tools: availableTools.map((tool) => ({
          name: tool.name,
          description: tool.description,
          inputSchema: tool.inputSchema,
        })),
      };
    });

    // Handle tool calls
    this.server.setRequestHandler(CallToolRequestSchema, async (request): Promise<{
      content: Array<{ type: 'text'; text: string }>;
      isError?: boolean;
    }> => {
      const { name, arguments: args } = request.params;

      this.log('tools/call', { name, args });

      const handler = toolHandlers[name];
      if (!handler) {
        return {
          content: [
            {
              type: 'text',
              text: `Unknown tool: ${name}. Use tools/list to see available tools.`,
            },
          ],
          isError: true,
        };
      }

      try {
        const result = await handler(args ?? {}, this.toolContext);
        this.log('Tool result', { name, isError: result.isError });
        return {
          content: result.content.map((c) => ({
            type: 'text' as const,
            text: c.text ?? '',
          })),
          isError: result.isError,
        };
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        this.log('Tool error', { name, error: message });
        return {
          content: [
            {
              type: 'text',
              text: `Error executing ${name}: ${message}`,
            },
          ],
          isError: true,
        };
      }
    });
  }

  /**
   * Start the MCP server with stdio transport
   */
  async start(): Promise<void> {
    if (this.isStarted) {
      throw new Error('Server is already started');
    }

    // Initialize client storage
    await this.client.start();

    // Connect via stdio transport
    const transport = new StdioServerTransport();
    await this.server.connect(transport);

    this.isStarted = true;
    this.log('TopGun MCP Server started on stdio');
  }

  /**
   * Start the MCP server with a custom transport
   */
  async startWithTransport(transport: { start(): Promise<void> }): Promise<void> {
    if (this.isStarted) {
      throw new Error('Server is already started');
    }

    // Initialize client storage
    await this.client.start();

    // Connect via provided transport
    await this.server.connect(transport as any);

    this.isStarted = true;
    this.log('TopGun MCP Server started with custom transport');
  }

  /**
   * Stop the server and cleanup resources
   */
  async stop(): Promise<void> {
    if (!this.isStarted) return;

    await this.server.close();

    if (!this.externalClient) {
      this.client.close();
    }

    this.isStarted = false;
    this.log('TopGun MCP Server stopped');
  }

  /**
   * Execute a tool directly (for testing)
   */
  async callTool(name: string, args: unknown): Promise<unknown> {
    const handler = toolHandlers[name];
    if (!handler) {
      throw new Error(`Unknown tool: ${name}`);
    }

    return handler(args, this.toolContext);
  }

  /**
   * Get the underlying MCP server instance
   */
  getServer(): Server {
    return this.server;
  }

  /**
   * Get the TopGun client instance
   */
  getClient(): TopGunClient {
    return this.client;
  }

  /**
   * Get resolved configuration
   */
  getConfig(): ResolvedMCPServerConfig {
    return { ...this.config };
  }

  /**
   * Debug logging
   */
  private log(message: string, data?: Record<string, unknown>): void {
    if (this.config.debug) {
      console.error(`[TopGunMCP] ${message}`, data ? JSON.stringify(data) : '');
    }
  }
}

/**
 * Simple in-memory storage adapter for MCP server use
 * Implements IStorageAdapter interface from @topgunbuild/client
 */
class InMemoryStorageAdapter implements IStorageAdapter {
  private data = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private opLog: OpLogEntry[] = [];
  private opLogIdCounter = 0;

  async initialize(_name: string): Promise<void> {
    // No-op
  }

  async close(): Promise<void> {
    this.data.clear();
    this.meta.clear();
    this.opLog = [];
  }

  async get(key: string): Promise<unknown | undefined> {
    return this.data.get(key);
  }

  async put(key: string, value: unknown): Promise<void> {
    this.data.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.data.delete(key);
  }

  async getAllKeys(): Promise<string[]> {
    return Array.from(this.data.keys());
  }

  async getMeta(key: string): Promise<unknown | undefined> {
    return this.meta.get(key);
  }

  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }

  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [key, value] of entries) {
      this.data.set(key, value);
    }
  }

  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = ++this.opLogIdCounter;
    this.opLog.push({ ...entry, id });
    return id;
  }

  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.opLog.filter((e) => e.synced === 0);
  }

  async markOpsSynced(lastId: number): Promise<void> {
    for (const op of this.opLog) {
      if (op.id !== undefined && op.id <= lastId) {
        op.synced = 1;
      }
    }
  }
}
