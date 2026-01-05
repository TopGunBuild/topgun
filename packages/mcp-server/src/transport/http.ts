/**
 * HTTP/SSE Transport for MCP Server
 *
 * Provides HTTP-based transport for web MCP clients
 */

import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { SSEServerTransport } from '@modelcontextprotocol/sdk/server/sse.js';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { createServer, type Server as HTTPServer } from 'node:http';
import { randomUUID } from 'node:crypto';
import type { TopGunMCPServer } from '../TopGunMCPServer';

/**
 * HTTP Server configuration
 */
export interface HTTPServerConfig {
  /**
   * Port to listen on
   * @default 3000
   */
  port?: number;

  /**
   * Host to bind to
   * @default '0.0.0.0'
   */
  host?: string;

  /**
   * CORS allowed origins
   * @default ['*']
   */
  corsOrigins?: string[];

  /**
   * Path for MCP endpoint
   * @default '/mcp'
   */
  mcpPath?: string;

  /**
   * Path for SSE events
   * @default '/mcp/events'
   */
  eventPath?: string;

  /**
   * Enable debug logging
   * @default false
   */
  debug?: boolean;
}

/**
 * Resolved HTTP server configuration
 */
interface ResolvedHTTPConfig {
  port: number;
  host: string;
  corsOrigins: string[];
  mcpPath: string;
  eventPath: string;
  debug: boolean;
}

const DEFAULT_HTTP_CONFIG: ResolvedHTTPConfig = {
  port: 3000,
  host: '0.0.0.0',
  corsOrigins: ['*'],
  mcpPath: '/mcp',
  eventPath: '/mcp/events',
  debug: false,
};

/**
 * HTTP Transport wrapper for MCP Server
 */
export class HTTPTransport {
  private readonly config: ResolvedHTTPConfig;
  private httpServer?: HTTPServer;
  private isRunning = false;
  private activeSessions = new Map<string, SSEServerTransport>();

  constructor(config: HTTPServerConfig = {}) {
    this.config = {
      port: config.port ?? DEFAULT_HTTP_CONFIG.port,
      host: config.host ?? DEFAULT_HTTP_CONFIG.host,
      corsOrigins: config.corsOrigins ?? DEFAULT_HTTP_CONFIG.corsOrigins,
      mcpPath: config.mcpPath ?? DEFAULT_HTTP_CONFIG.mcpPath,
      eventPath: config.eventPath ?? DEFAULT_HTTP_CONFIG.eventPath,
      debug: config.debug ?? DEFAULT_HTTP_CONFIG.debug,
    };
  }

  /**
   * Start HTTP server with MCP transport
   */
  async start(mcpServer: TopGunMCPServer): Promise<void> {
    if (this.isRunning) {
      throw new Error('HTTP transport is already running');
    }

    this.httpServer = createServer((req, res) => {
      this.handleRequest(req, res, mcpServer);
    });

    return new Promise((resolve, reject) => {
      this.httpServer!.on('error', reject);

      this.httpServer!.listen(this.config.port, this.config.host, () => {
        this.isRunning = true;
        this.log(`HTTP transport listening on ${this.config.host}:${this.config.port}`);
        resolve();
      });
    });
  }

  /**
   * Stop HTTP server
   */
  async stop(): Promise<void> {
    if (!this.isRunning || !this.httpServer) return;

    // Close all active sessions
    for (const [sessionId, transport] of this.activeSessions) {
      try {
        await transport.close();
      } catch {
        // Ignore errors during cleanup
      }
      this.activeSessions.delete(sessionId);
    }

    return new Promise((resolve) => {
      this.httpServer!.close(() => {
        this.isRunning = false;
        this.log('HTTP transport stopped');
        resolve();
      });
    });
  }

  /**
   * Handle incoming HTTP request
   */
  private async handleRequest(
    req: IncomingMessage,
    res: ServerResponse,
    mcpServer: TopGunMCPServer
  ): Promise<void> {
    // Set CORS headers
    this.setCorsHeaders(req, res);

    // Handle preflight
    if (req.method === 'OPTIONS') {
      res.writeHead(204);
      res.end();
      return;
    }

    const url = new URL(req.url || '/', `http://${req.headers.host}`);
    const pathname = url.pathname;

    this.log(`${req.method} ${pathname}`);

    // Health check endpoint
    if (pathname === '/health') {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ status: 'ok', timestamp: new Date().toISOString() }));
      return;
    }

    // MCP info endpoint
    if (pathname === this.config.mcpPath && req.method === 'GET') {
      const config = mcpServer.getConfig();
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(
        JSON.stringify({
          name: config.name,
          version: config.version,
          transport: 'http+sse',
          mcpPath: this.config.mcpPath,
          eventPath: this.config.eventPath,
        })
      );
      return;
    }

    // SSE connection for MCP
    if (pathname === this.config.eventPath && req.method === 'GET') {
      await this.handleSSEConnection(req, res, mcpServer);
      return;
    }

    // MCP POST request (for clients that don't support SSE)
    if (pathname === this.config.mcpPath && req.method === 'POST') {
      await this.handleMCPRequest(req, res, mcpServer);
      return;
    }

    // Not found
    res.writeHead(404, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ error: 'Not found' }));
  }

  /**
   * Handle SSE connection for real-time MCP
   */
  private async handleSSEConnection(
    _req: IncomingMessage,
    res: ServerResponse,
    mcpServer: TopGunMCPServer
  ): Promise<void> {
    const sessionId = randomUUID();

    this.log(`New SSE session: ${sessionId}`);

    // Create SSE transport
    const transport = new SSEServerTransport(this.config.mcpPath, res);
    this.activeSessions.set(sessionId, transport);

    try {
      // Connect to MCP server
      await mcpServer.getServer().connect(transport);

      // Wait for connection to close
      await new Promise<void>((resolve) => {
        res.on('close', () => {
          this.log(`SSE session closed: ${sessionId}`);
          this.activeSessions.delete(sessionId);
          resolve();
        });
      });
    } catch (error) {
      this.log(`SSE session error: ${sessionId}`, error);
      this.activeSessions.delete(sessionId);
    }
  }

  /**
   * Handle stateless MCP POST request
   */
  private async handleMCPRequest(
    req: IncomingMessage,
    res: ServerResponse,
    mcpServer: TopGunMCPServer
  ): Promise<void> {
    try {
      // Read body
      const body = await this.readBody(req);
      const request = JSON.parse(body);

      this.log('MCP request', request);

      // Extract tool call from request
      if (request.method === 'tools/call') {
        const { name, arguments: args } = request.params || {};

        if (!name) {
          res.writeHead(400, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: 'Missing tool name' }));
          return;
        }

        const result = await mcpServer.callTool(name, args);

        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ result }));
      } else {
        res.writeHead(400, { 'Content-Type': 'application/json' });
        res.end(
          JSON.stringify({
            error: 'Unsupported method. Use SSE transport for full MCP support.',
          })
        );
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.log('MCP request error', error);

      res.writeHead(500, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: message }));
    }
  }

  /**
   * Read request body
   */
  private readBody(req: IncomingMessage): Promise<string> {
    return new Promise((resolve, reject) => {
      let body = '';
      req.on('data', (chunk) => (body += chunk));
      req.on('end', () => resolve(body));
      req.on('error', reject);
    });
  }

  /**
   * Set CORS headers
   */
  private setCorsHeaders(req: IncomingMessage, res: ServerResponse): void {
    const origin = req.headers.origin || '*';
    const allowedOrigin = this.config.corsOrigins.includes('*')
      ? '*'
      : this.config.corsOrigins.includes(origin)
        ? origin
        : this.config.corsOrigins[0];

    res.setHeader('Access-Control-Allow-Origin', allowedOrigin);
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');
    res.setHeader('Access-Control-Max-Age', '86400');
  }

  /**
   * Debug logging
   */
  private log(message: string, data?: unknown): void {
    if (this.config.debug) {
      console.error(`[HTTPTransport] ${message}`, data ? JSON.stringify(data) : '');
    }
  }

  /**
   * Get current session count
   */
  getSessionCount(): number {
    return this.activeSessions.size;
  }

  /**
   * Check if running
   */
  isActive(): boolean {
    return this.isRunning;
  }
}

/**
 * Create and start an HTTP transport
 */
export async function createHTTPServer(
  mcpServer: TopGunMCPServer,
  config?: HTTPServerConfig
): Promise<HTTPTransport> {
  const transport = new HTTPTransport(config);
  await transport.start(mcpServer);
  return transport;
}
