/**
 * HTTP/SSE Transport for MCP Server
 *
 * Provides HTTP-based transport for web MCP clients
 */

import { SSEServerTransport } from '@modelcontextprotocol/sdk/server/sse.js';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { createServer, type Server as HTTPServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { isIP } from 'node:net';
import { logger } from '../logger';
import { parsePort } from '../port';
import { createHash, randomUUID, timingSafeEqual } from 'node:crypto';
import type { TopGunMCPServer } from '../TopGunMCPServer';

/**
 * HTTP Server configuration
 */
export interface HTTPServerConfig {
  /**
   * Port to listen on: an integer from 1 to 65535.
   * @default 3000
   */
  port?: number;

  /**
   * Host to bind to: an IP literal. A non-loopback address is refused unless
   * `allowPublicBind` is true.
   * @default '127.0.0.1'
   */
  host?: string;

  /**
   * Allow binding a non-loopback `host`, which makes the transport reachable from the network.
   * @default false
   */
  allowPublicBind?: boolean;

  /**
   * Secret every request must present as `Authorization: Bearer <token>`: at least 32
   * characters of the RFC 6750 b64token alphabet.
   */
  bearerToken: string;

  /**
   * Origins allowed to call the transport from a browser, each a serialized origin such as
   * `https://app.example.com`. A request whose `Origin` is not listed gets 403; `*` is refused.
   * @default []
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
  // Only the digest is kept, so the secret itself is not held by the transport.
  bearerDigest: Buffer;
  corsOrigins: string[];
  mcpPath: string;
  eventPath: string;
  debug: boolean;
}

const DEFAULT_HTTP_CONFIG = {
  port: 3000,
  host: '127.0.0.1',
  mcpPath: '/mcp',
  eventPath: '/mcp/events',
  debug: false,
};

const BEARER_TOKEN_PATTERN = /^[A-Za-z0-9\-._~+/]+=*$/;
const BEARER_TOKEN_MIN_LENGTH = 32;

/**
 * A setting the transport refuses to start with. The message starts with `field`, so a caller
 * that names the setting differently (the CLI) can substitute its own name.
 */
export class HTTPTransportConfigError extends Error {
  constructor(
    readonly field: 'bearerToken' | 'host' | 'port' | 'corsOrigins',
    message: string,
  ) {
    super(message);
    this.name = 'HTTPTransportConfigError';
  }
}

/**
 * True only for an IPv4 literal in 127.0.0.0/8 and the literal `::1`. Every other spelling,
 * IPv4-mapped IPv6 included, counts as non-loopback so that an unusual spelling never widens
 * the bind silently.
 */
export function isLoopbackHost(host: string): boolean {
  // net.isIP accepts no leading zeros in IPv4, so the prefix decides the first octet.
  return host === '::1' || (isIP(host) === 4 && host.startsWith('127.'));
}

function sha256(value: string): Buffer {
  return createHash('sha256').update(value).digest();
}

function resolveConfig(given: HTTPServerConfig | undefined): ResolvedHTTPConfig {
  // A JavaScript caller may pass nothing; that is a missing bearerToken, not a TypeError.
  const config: Partial<HTTPServerConfig> = given ?? {};
  const token: unknown = config.bearerToken;
  if (
    typeof token !== 'string' ||
    token.length < BEARER_TOKEN_MIN_LENGTH ||
    !BEARER_TOKEN_PATTERN.test(token)
  ) {
    throw new HTTPTransportConfigError(
      'bearerToken',
      `bearerToken must be at least ${BEARER_TOKEN_MIN_LENGTH} characters of [A-Za-z0-9-._~+/] with optional trailing '='; generate one with: openssl rand -base64 32`,
    );
  }

  const host = config.host ?? DEFAULT_HTTP_CONFIG.host;
  // A name may resolve to a non-loopback address, so only a literal can be judged before the bind.
  if (isIP(host) === 0) {
    throw new HTTPTransportConfigError(
      'host',
      `host must be an IP address (e.g. 127.0.0.1 or ::1), got '${host}'`,
    );
  }
  if (!isLoopbackHost(host) && config.allowPublicBind !== true) {
    throw new HTTPTransportConfigError(
      'host',
      `host ${host} is not a loopback address; the MCP HTTP transport binds loopback only unless allowPublicBind is set`,
    );
  }

  let port: number;
  try {
    port = parsePort(config.port ?? DEFAULT_HTTP_CONFIG.port);
  } catch (error) {
    throw new HTTPTransportConfigError('port', `port ${(error as Error).message}`);
  }

  const corsOrigins = config.corsOrigins ?? [];
  for (const entry of corsOrigins) {
    if (entry === '*') {
      throw new HTTPTransportConfigError(
        'corsOrigins',
        "corsOrigins must not contain '*': every request carries a bearer credential",
      );
    }
    const origin = serializedOrigin(entry);
    if (origin !== entry) {
      // Named, not substituted: the allowlist holds exactly what the operator wrote.
      const hint = origin && origin !== 'null' ? `; write '${origin}'` : '';
      throw new HTTPTransportConfigError(
        'corsOrigins',
        `corsOrigins entries must be serialized origins (scheme://host[:port], no path, no trailing slash, no default port), got '${String(entry)}'${hint}`,
      );
    }
  }

  return {
    port,
    host,
    bearerDigest: sha256(token),
    corsOrigins: [...corsOrigins],
    mcpPath: config.mcpPath ?? DEFAULT_HTTP_CONFIG.mcpPath,
    eventPath: config.eventPath ?? DEFAULT_HTTP_CONFIG.eventPath,
    debug: config.debug ?? DEFAULT_HTTP_CONFIG.debug,
  };
}

function serializedOrigin(entry: unknown): string | undefined {
  if (typeof entry !== 'string') return undefined;
  try {
    return new URL(entry).origin;
  } catch {
    return undefined;
  }
}

/**
 * HTTP Transport wrapper for MCP Server
 */
export class HTTPTransport {
  private readonly config: ResolvedHTTPConfig;
  private httpServer?: HTTPServer;
  private isRunning = false;
  private activeSessions = new Map<string, SSEServerTransport>();

  /**
   * @throws {HTTPTransportConfigError} on a missing config or `bearerToken`, a malformed
   * `bearerToken`, a `port` that is not an integer from 1 to 65535, a hostname or a
   * non-loopback `host` without `allowPublicBind`, and a `corsOrigins` entry that is `*` or not
   * a serialized origin.
   */
  constructor(config: HTTPServerConfig) {
    this.config = resolveConfig(config);
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
      // Force-close keep-alive connections so close() completes immediately instead of
      // waiting for each client to finish its TCP keep-alive idle timeout (up to 5s),
      // which would block Node.js exit in test environments.
      this.httpServer!.closeAllConnections?.();

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
    mcpServer: TopGunMCPServer,
  ): Promise<void> {
    // The gate runs before routing, before the body is read and before an SSE session is
    // connected, so a refused request runs nothing. The bearer is checked before the origin, so
    // an unauthenticated caller gets the same 401 whatever Origin it sends and cannot probe the
    // allowlist.
    res.setHeader('Vary', 'Origin');
    const origin = req.headers.origin;
    const allowedOrigin =
      origin !== undefined && this.config.corsOrigins.includes(origin) ? origin : undefined;

    // Browsers send no credentials on a preflight, so it is answered without the bearer check;
    // it is 204 for every origin, and only an allowlisted one gets CORS headers.
    if (req.method === 'OPTIONS') {
      if (allowedOrigin) this.setCorsHeaders(res, allowedOrigin);
      res.writeHead(204);
      res.end();
      return;
    }

    if (!this.isAuthorized(req.headers.authorization)) {
      res.writeHead(401, { 'Content-Type': 'application/json', 'WWW-Authenticate': 'Bearer' });
      res.end(JSON.stringify({ error: 'unauthorized' }));
      return;
    }

    if (origin !== undefined && !allowedOrigin) {
      res.writeHead(403, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'origin not allowed' }));
      return;
    }
    if (allowedOrigin) this.setCorsHeaders(res, allowedOrigin);

    // A fixed base: only the path is used, and a malformed Host header must not make URL parsing throw.
    const pathname = new URL(req.url || '/', 'http://localhost').pathname;

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
          // Only stateless tools/call over POST works; SSE session messaging does not.
          transport: 'http',
          mcpPath: this.config.mcpPath,
        }),
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
    mcpServer: TopGunMCPServer,
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
    mcpServer: TopGunMCPServer,
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
            error: 'Unsupported method: only tools/call is supported over HTTP.',
          }),
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

  private setCorsHeaders(res: ServerResponse, origin: string): void {
    res.setHeader('Access-Control-Allow-Origin', origin);
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');
    res.setHeader('Access-Control-Max-Age', '86400');
  }

  /**
   * `Authorization: Bearer <token>` with the scheme matched case-insensitively (RFC 7235).
   * Both sides are hashed so timingSafeEqual compares equal lengths whatever the caller sent.
   */
  private isAuthorized(header: string | undefined): boolean {
    const match = header === undefined ? null : /^(\S+) +(\S+)$/.exec(header);
    if (!match || match[1].toLowerCase() !== 'bearer') return false;
    return timingSafeEqual(sha256(match[2]), this.config.bearerDigest);
  }

  /**
   * Debug logging
   */
  private log(message: string, data?: unknown): void {
    if (this.config.debug) {
      logger.debug({ data, context: 'HTTPTransport' }, message);
    }
  }

  /**
   * The bound address, or null before start() and after stop().
   */
  address(): AddressInfo | null {
    const bound = this.httpServer?.address();
    return bound && typeof bound === 'object' ? bound : null;
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
  config: HTTPServerConfig,
): Promise<HTTPTransport> {
  const transport = new HTTPTransport(config);
  await transport.start(mcpServer);
  return transport;
}
