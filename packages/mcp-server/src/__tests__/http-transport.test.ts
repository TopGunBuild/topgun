/**
 * HTTP Transport Integration Tests
 *
 * Tests HTTP transport endpoints with real HTTP requests:
 * - HTTPTransport with real Node.js HTTP server
 * - TopGunMCPServer integration
 * - bearer gate, origin allowlist and error handling
 */

import { TopGunMCPServer } from '../TopGunMCPServer';
import {
  HTTPTransport,
  HTTPTransportConfigError,
  createHTTPServer,
  type HTTPServerConfig,
} from '../transport/http';
import * as http from 'node:http';
import * as net from 'node:net';

// 48 characters of the b64token alphabet; every helper request sends it unless a test overrides.
const TOKEN = 'test-token-0123456789-abcdefghijklmnopqrstuvwxyz';

// The transport refuses port 0, so each start takes a port the OS just handed out.
let port = 0;

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.on('error', reject);
    probe.listen(0, '127.0.0.1', () => {
      const { port: free } = probe.address() as net.AddressInfo;
      probe.close(() => resolve(free));
    });
  });
}

// Mock the WebSocket used by TopGunMCPServer's internal TopGunClient so the client
// does not open a real undici connection to ws://localhost:8080. Without a mock, the
// real socket's DNS/TCP handles persist after client.close() and keep Jest alive.
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

/**
 * Helper to make HTTP requests. Sends `Authorization: Bearer TOKEN` unless `auth` overrides it
 * (`null` sends no Authorization header).
 */
function makeRequest(options: {
  method: string;
  path: string;
  headers?: Record<string, string>;
  auth?: string | null;
  body?: string;
}): Promise<{
  statusCode: number;
  headers: http.IncomingHttpHeaders;
  body: string;
}> {
  const auth = options.auth === undefined ? `Bearer ${TOKEN}` : options.auth;
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        // The transport binds 127.0.0.1; 'localhost' may resolve to ::1 first.
        hostname: '127.0.0.1',
        port,
        method: options.method,
        path: options.path,
        // 'connection: close' disables HTTP keep-alive on each request so the
        // socket is destroyed immediately after the response — without this, the
        // global http.Agent keeps sockets in its keep-alive pool for ~5s and Jest
        // sees them as open handles after all tests finish.
        headers: {
          connection: 'close',
          ...(auth === null ? {} : { authorization: auth }),
          ...(options.headers || {}),
        },
        // Must exceed the queryOnce settle timeout (DEFAULT_QUERY_ONCE_TIMEOUT_MS,
        // 5000ms): offline read tools resolve a not-settled message only after that
        // settle wait, so a shorter HTTP timeout would race and flake.
        timeout: 12000,
      },
      (res) => {
        let body = '';
        res.on('data', (chunk) => (body += chunk));
        res.on('end', () => {
          resolve({
            statusCode: res.statusCode || 0,
            headers: res.headers,
            body,
          });
        });
        res.on('error', reject);
      },
    );

    req.on('error', reject);
    req.on('timeout', () => {
      req.destroy();
      reject(new Error('Request timeout'));
    });

    if (options.body) {
      req.write(options.body);
    }

    req.end();
  });
}

/**
 * Opens GET /mcp/events and resolves on the response head; `close()` drops the stream.
 */
function openEvents(auth: string | null): Promise<{
  statusCode: number;
  contentType: string | undefined;
  close: () => Promise<void>;
}> {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port,
        method: 'GET',
        path: '/mcp/events',
        headers: { connection: 'close', ...(auth === null ? {} : { authorization: auth }) },
      },
      (res) => {
        resolve({
          statusCode: res.statusCode || 0,
          contentType: res.headers['content-type'],
          close: () => {
            req.destroy();
            return closed;
          },
        });
        res.resume();
      },
    );
    // Registered up front: a refused (already ended) response may close before close() is called.
    const closed = new Promise<void>((done) => req.on('close', () => done()));
    req.on('error', reject);
    req.end();
  });
}

function mutateBody(key: string): string {
  return JSON.stringify({
    method: 'tools/call',
    params: {
      name: 'topgun_mutate',
      arguments: { map: 'gate', operation: 'set', key, data: { value: 'x' } },
    },
  });
}

function configError(config: unknown): HTTPTransportConfigError {
  try {
    new HTTPTransport(config as HTTPServerConfig);
  } catch (error) {
    expect(error).toBeInstanceOf(HTTPTransportConfigError);
    return error as HTTPTransportConfigError;
  }
  throw new Error(`HTTPTransport accepted ${JSON.stringify(config)}`);
}

describe('HTTP Transport', () => {
  let transport: HTTPTransport;
  let server: TopGunMCPServer;

  async function startTransport(config: Partial<HTTPServerConfig> = {}): Promise<void> {
    transport = new HTTPTransport({ port: await freePort(), bearerToken: TOKEN, ...config });
    await transport.start(server);
    port = transport.address()!.port;
  }

  async function restart(
    config: Partial<HTTPServerConfig>,
    serverConfig?: ConstructorParameters<typeof TopGunMCPServer>[0],
  ): Promise<void> {
    await transport.stop();
    const client = server.getClient();
    if (client) {
      await client.close();
    }
    server = new TopGunMCPServer(serverConfig);
    await startTransport(config);
  }

  beforeEach(async () => {
    server = new TopGunMCPServer();
    await startTransport();

    // Give server a moment to fully start
    await new Promise((resolve) => setTimeout(resolve, 100));
  });

  afterEach(async () => {
    if (transport) {
      await transport.stop();
    }
    if (server) {
      const client = server.getClient();
      if (client) {
        await client.close();
      }
    }
  });

  describe('Endpoints', () => {
    it('should respond to GET /health with status ok', async () => {
      const response = await makeRequest({
        method: 'GET',
        path: '/health',
      });

      expect(response.statusCode).toBe(200);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.status).toBe('ok');
      expect(data.timestamp).toBeDefined();
      expect(new Date(data.timestamp).getTime()).toBeGreaterThan(0);
    });

    it('should respond to GET /mcp with server info', async () => {
      const response = await makeRequest({
        method: 'GET',
        path: '/mcp',
      });

      expect(response.statusCode).toBe(200);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.name).toBe('topgun-mcp-server');
      expect(data.version).toBe('1.0.0');
      // Only stateless tools/call works over HTTP, so SSE is not advertised.
      expect(data.transport).toBe('http');
      expect(data.mcpPath).toBe('/mcp');
      expect(data.eventPath).toBeUndefined();
    });

    it('should execute tool via POST /mcp with tools/call', async () => {
      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          method: 'tools/call',
          params: {
            name: 'topgun_stats',
            arguments: {},
          },
        }),
      });

      expect(response.statusCode).toBe(200);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.result).toBeDefined();
      expect(data.result.content).toBeDefined();
      expect(data.result.content[0].text).toContain('Connection');
    });

    it('should answer OPTIONS with no Origin with 204 and no CORS headers', async () => {
      const response = await makeRequest({
        method: 'OPTIONS',
        path: '/mcp',
        auth: null,
      });

      expect(response.statusCode).toBe(204);
      expect(response.headers['access-control-allow-origin']).toBeUndefined();
      expect(response.headers['access-control-allow-methods']).toBeUndefined();
    });

    it('should survive a Host header that is not a valid URL authority', async () => {
      const statusLine = await new Promise<string>((resolve, reject) => {
        const socket = net.connect(port, '127.0.0.1', () => {
          socket.write(
            `GET /health HTTP/1.1\r\nHost: a b\r\nAuthorization: Bearer ${TOKEN}\r\nConnection: close\r\n\r\n`,
          );
        });
        let data = '';
        socket.on('data', (chunk) => (data += chunk));
        socket.on('close', () => resolve(data.split('\r\n')[0]));
        socket.on('error', reject);
      });
      expect(statusLine).toBe('HTTP/1.1 200 OK');
      expect((await makeRequest({ method: 'GET', path: '/health' })).statusCode).toBe(200);
    });

    it('should return 404 for unknown routes', async () => {
      const response = await makeRequest({
        method: 'GET',
        path: '/unknown/path',
      });

      expect(response.statusCode).toBe(404);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.error).toBe('Not found');
    });
  });

  describe('Config', () => {
    it('T16: refuses a port that is not an integer from 1 to 65535', () => {
      for (const bad of [0, -1, 65536, 1.5, NaN, '3e3' as unknown as number]) {
        const error = configError({ bearerToken: TOKEN, port: bad });
        expect(error.field).toBe('port');
        expect(error.message).toMatch(/^port must be an integer from 1 to 65535, got /);
      }
      expect(() => new HTTPTransport({ bearerToken: TOKEN, port: 65535 })).not.toThrow();
    });

    it('T17: refuses a missing config with HTTPTransportConfigError, not a TypeError', async () => {
      expect(configError(undefined).field).toBe('bearerToken');
      expect(configError(null).field).toBe('bearerToken');
      await expect(
        createHTTPServer(server, undefined as unknown as HTTPServerConfig),
      ).rejects.toBeInstanceOf(HTTPTransportConfigError);
    });
  });

  describe('Bind', () => {
    it('T1: binds 127.0.0.1 when no host is given', () => {
      expect(transport.address()?.address).toBe('127.0.0.1');
    });

    it('T2: refuses non-loopback hosts and hostnames without allowPublicBind', () => {
      for (const host of ['0.0.0.0', '::', '192.168.1.10', '127::1', 'localhost']) {
        expect(configError({ bearerToken: TOKEN, host }).field).toBe('host');
      }
      for (const host of ['::1', '127.0.0.2']) {
        expect(() => new HTTPTransport({ bearerToken: TOKEN, host })).not.toThrow();
      }
      expect(
        () => new HTTPTransport({ bearerToken: TOKEN, host: '0.0.0.0', allowPublicBind: true }),
      ).not.toThrow();
      // A name may resolve anywhere, so allowPublicBind does not admit one either.
      const named = configError({ bearerToken: TOKEN, host: 'localhost', allowPublicBind: true });
      expect(named.message).toContain('must be an IP address');
    });

    it('T14: refuses every other IPv6 spelling of loopback, IPv4-mapped forms included', () => {
      for (const host of [
        '0:0:0:0:0:0:0:1',
        '::ffff:7f00:1',
        '::ffff:127.0.0.1',
        '::ffff:10.0.0.1',
      ]) {
        expect(configError({ bearerToken: TOKEN, host }).field).toBe('host');
      }
    });
  });

  describe('Bearer gate', () => {
    it('T3: refuses every route without Authorization and runs no tool', async () => {
      const callTool = jest.spyOn(server, 'callTool');
      const requests = [
        { method: 'GET', path: '/health' },
        { method: 'GET', path: '/mcp' },
        { method: 'GET', path: '/mcp/events' },
        { method: 'POST', path: '/mcp', body: mutateBody('unauthorized') },
        { method: 'GET', path: '/unknown' },
      ];
      for (const request of requests) {
        const response = await makeRequest({
          ...request,
          headers: { 'Content-Type': 'application/json' },
          auth: null,
        });
        expect([request.path, response.statusCode]).toEqual([request.path, 401]);
        expect(response.headers['www-authenticate']).toBe('Bearer');
        expect(JSON.parse(response.body)).toEqual({ error: 'unauthorized' });
      }
      expect(callTool).not.toHaveBeenCalled();
      expect(transport.getSessionCount()).toBe(0);
    });

    it('T4: refuses a wrong, truncated or extended token, another scheme and an empty token', async () => {
      const wrong = TOKEN.slice(0, -1) + (TOKEN.endsWith('z') ? 'y' : 'z');
      for (const auth of [
        `Bearer ${wrong}`,
        `Bearer ${TOKEN.slice(0, -1)}`,
        `Bearer ${TOKEN}x`,
        `Basic ${TOKEN}`,
        'Bearer ',
        'Bearer',
        `Bearer ${TOKEN} extra`,
      ]) {
        const response = await makeRequest({ method: 'GET', path: '/health', auth });
        expect([auth, response.statusCode]).toEqual([auth, 401]);
      }
    });

    it('T5: accepts the scheme case-insensitively', async () => {
      for (const scheme of ['bearer', 'BEARER']) {
        const response = await makeRequest({
          method: 'GET',
          path: '/health',
          auth: `${scheme} ${TOKEN}`,
        });
        expect(response.statusCode).toBe(200);
      }
    });

    it('T6: checks the bearer on every SSE connection before a session exists', async () => {
      const refused = await openEvents(null);
      expect(refused.statusCode).toBe(401);
      expect(transport.getSessionCount()).toBe(0);
      await refused.close();

      const first = await openEvents(`Bearer ${TOKEN}`);
      expect(first.statusCode).toBe(200);
      expect(first.contentType).toContain('text/event-stream');
      await first.close();

      const second = await openEvents(`Bearer ${TOKEN}`);
      expect(second.statusCode).toBe(200);
      await second.close();

      const wrong = await openEvents(`Bearer ${TOKEN}x`);
      expect(wrong.statusCode).toBe(401);
      await wrong.close();
    });

    it('T13: answers 401 before the request body is read', async () => {
      const statusCode = await new Promise<number>((resolve, reject) => {
        let answered = false;
        const req = http.request({
          hostname: '127.0.0.1',
          port,
          method: 'POST',
          path: '/mcp',
          headers: { connection: 'close', 'content-type': 'application/json' },
        });
        // A gate that reads the body first never answers while the body is open.
        const deadline = setTimeout(() => {
          req.destroy();
          reject(new Error('no response within 2 s while the request body was still open'));
        }, 2000);
        req.on('response', (res) => {
          answered = true;
          clearTimeout(deadline);
          res.resume();
          req.end();
          resolve(res.statusCode || 0);
        });
        req.on('error', (error: NodeJS.ErrnoException) => {
          // The server may close the socket once it has answered; that is not a failure.
          if (answered && (error.code === 'ECONNRESET' || error.code === 'EPIPE')) return;
          clearTimeout(deadline);
          reject(error);
        });
        req.flushHeaders();
        req.write('{"method":"tools/call",');
      });
      expect(statusCode).toBe(401);
    });

    it('T7: refuses an absent, short, whitespace or malformed secret without echoing it', () => {
      const values = [
        undefined,
        '',
        '   ',
        'a'.repeat(31),
        `${'b'.repeat(40)}\n`,
        `${'c'.repeat(20)} ${'d'.repeat(20)}`,
      ];
      for (const bearerToken of values) {
        const error = configError({ bearerToken });
        expect(error.field).toBe('bearerToken');
        if (bearerToken && bearerToken.trim()) {
          expect(error.message).not.toContain(bearerToken.trim());
        }
      }
    });
  });

  describe('CORS', () => {
    it('T8: refuses an authenticated request from an origin that is not allowlisted, null included', async () => {
      for (const origin of ['http://evil.example', 'null']) {
        const response = await makeRequest({
          method: 'GET',
          path: '/health',
          headers: { Origin: origin },
        });
        expect([origin, response.statusCode]).toEqual([origin, 403]);
        expect(response.headers['access-control-allow-origin']).toBeUndefined();
        expect(JSON.parse(response.body)).toEqual({ error: 'origin not allowed' });
      }

      // A preflight is 204 for every origin; only an allowlisted one gets CORS headers.
      const preflight = await makeRequest({
        method: 'OPTIONS',
        path: '/mcp',
        headers: { Origin: 'http://evil.example' },
        auth: null,
      });
      expect(preflight.statusCode).toBe(204);
      expect(preflight.headers['access-control-allow-origin']).toBeUndefined();
      expect(preflight.headers['access-control-allow-methods']).toBeUndefined();
      expect(preflight.headers['access-control-allow-headers']).toBeUndefined();
    });

    it('T8b: answers an unauthenticated request the same whatever Origin it sends', async () => {
      await restart({ corsOrigins: ['http://allowed.example'] });

      const seen = [];
      for (const origin of [undefined, 'http://evil.example', 'http://allowed.example', 'null']) {
        const response = await makeRequest({
          method: 'GET',
          path: '/health',
          headers: origin === undefined ? {} : { Origin: origin },
          auth: null,
        });
        seen.push({
          status: response.statusCode,
          body: response.body,
          wwwAuthenticate: response.headers['www-authenticate'],
          acao: response.headers['access-control-allow-origin'],
        });
      }
      expect(seen[0]).toEqual({
        status: 401,
        body: JSON.stringify({ error: 'unauthorized' }),
        wwwAuthenticate: 'Bearer',
        acao: undefined,
      });
      for (const other of seen.slice(1)) {
        expect(other).toEqual(seen[0]);
      }
    });

    it('T9: answers an allowlisted origin with its own ACAO, and its preflight without a token', async () => {
      await restart({ corsOrigins: ['http://allowed.example'] });

      const response = await makeRequest({
        method: 'GET',
        path: '/health',
        headers: { Origin: 'http://allowed.example' },
      });
      expect(response.statusCode).toBe(200);
      expect(response.headers['access-control-allow-origin']).toBe('http://allowed.example');
      expect(response.headers['vary']).toContain('Origin');

      const preflight = await makeRequest({
        method: 'OPTIONS',
        path: '/mcp',
        headers: { Origin: 'http://allowed.example' },
        auth: null,
      });
      expect(preflight.statusCode).toBe(204);
      expect(preflight.headers['access-control-allow-origin']).toBe('http://allowed.example');
      expect(preflight.headers['access-control-allow-headers']).toContain('Authorization');
    });

    it('T10: refuses * and entries that are not serialized origins', () => {
      for (const corsOrigins of [['*'], ['http://a.example/'], ['null']]) {
        expect(configError({ bearerToken: TOKEN, corsOrigins }).field).toBe('corsOrigins');
      }
    });

    it('T10b: refuses a default port or an upper-case host, naming the serialized form', () => {
      for (const [entry, serialized] of [
        ['https://app.example.com:443', 'https://app.example.com'],
        ['http://app.example.com:80', 'http://app.example.com'],
        ['http://App.Example', 'http://app.example'],
      ]) {
        const error = configError({ bearerToken: TOKEN, corsOrigins: [entry] });
        expect(error.field).toBe('corsOrigins');
        expect(error.message).toContain(`write '${serialized}'`);
      }
      // A non-default port is part of the origin and is kept.
      expect(
        () =>
          new HTTPTransport({ bearerToken: TOKEN, corsOrigins: ['https://app.example.com:8443'] }),
      ).not.toThrow();
    });

    it('T11: sends no ACAO under the default config', async () => {
      const response = await makeRequest({ method: 'GET', path: '/health' });

      expect(response.statusCode).toBe(200);
      expect(response.headers['access-control-allow-origin']).toBeUndefined();
      expect(response.headers['vary']).toContain('Origin');
    });
  });

  describe('Error Handling', () => {
    it('should return 500 for invalid JSON body', async () => {
      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: {
          'Content-Type': 'application/json',
        },
        body: 'invalid json {',
      });

      expect(response.statusCode).toBe(500);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.error).toBeDefined();
    });

    it('should return 400 for missing tool name', async () => {
      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          method: 'tools/call',
          params: {
            // Missing 'name' field
            arguments: {},
          },
        }),
      });

      expect(response.statusCode).toBe(400);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.error).toContain('Missing tool name');
    });

    it('should return 400 for unknown method', async () => {
      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          method: 'unknown/method',
          params: {},
        }),
      });

      expect(response.statusCode).toBe(400);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.error).toContain('Unsupported method');
    });

    it('should handle tool execution errors gracefully', async () => {
      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          method: 'tools/call',
          params: {
            name: 'topgun_query',
            arguments: {
              map: 123, // Invalid type
            },
          },
        }),
      });

      expect(response.statusCode).toBe(200);
      expect(response.headers['content-type']).toContain('application/json');

      const data = JSON.parse(response.body);
      expect(data.result).toBeDefined();
      expect(data.result.isError).toBe(true);
      expect(data.result.content[0].text).toContain('Invalid arguments');
    });
  });

  describe('Transport Lifecycle', () => {
    it('should track active state correctly', async () => {
      expect(transport.isActive()).toBe(true);

      await transport.stop();
      expect(transport.isActive()).toBe(false);
    });

    it('should prevent double start', async () => {
      await expect(transport.start(server)).rejects.toThrow('already running');
    });

    it('should handle graceful shutdown', async () => {
      const healthResponse = await makeRequest({
        method: 'GET',
        path: '/health',
      });
      expect(healthResponse.statusCode).toBe(200);

      await transport.stop();

      // After stop, requests should fail
      await expect(
        makeRequest({
          method: 'GET',
          path: '/health',
        }),
      ).rejects.toThrow();
    });
  });

  describe('MCP Tool Integration', () => {
    it('should execute query tool via HTTP POST', async () => {
      // First, create some data
      await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          method: 'tools/call',
          params: {
            name: 'topgun_mutate',
            arguments: {
              map: 'test',
              operation: 'set',
              key: 'key1',
              data: { value: 'hello' },
            },
          },
        }),
      });

      // Query the data
      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          method: 'tools/call',
          params: {
            name: 'topgun_query',
            arguments: { map: 'test' },
          },
        }),
      });

      expect(response.statusCode).toBe(200);

      const data = JSON.parse(response.body);
      // No reachable server in this harness: server-truth queryOnce cannot settle,
      // so the tool returns an explicit not-settled message over HTTP, never a
      // silent empty or stale local data.
      expect(data.result.content[0].text).toContain('not settled');
      expect(data.result.content[0].text).not.toContain('No results found');
    }, 20000);

    it('should execute list_maps tool via HTTP POST', async () => {
      // Create server with allowed maps
      await transport.stop();
      const client = server.getClient();
      if (client) {
        await client.close();
      }
      server = new TopGunMCPServer({
        allowedMaps: ['map1', 'map2'],
      });
      await startTransport();

      const response = await makeRequest({
        method: 'POST',
        path: '/mcp',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          method: 'tools/call',
          params: {
            name: 'topgun_list_maps',
            arguments: {},
          },
        }),
      });

      expect(response.statusCode).toBe(200);

      const data = JSON.parse(response.body);
      expect(data.result.content[0].text).toContain('map1');
      expect(data.result.content[0].text).toContain('map2');
    });
  });
});
