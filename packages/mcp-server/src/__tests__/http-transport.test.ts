/**
 * HTTP Transport Integration Tests
 *
 * Tests HTTP transport endpoints with real HTTP requests:
 * - HTTPTransport with real Node.js HTTP server
 * - TopGunMCPServer integration
 * - CORS headers and error handling
 */

import { TopGunMCPServer } from '../TopGunMCPServer';
import { HTTPTransport } from '../transport/http';
import * as http from 'node:http';

// Fixed test port to avoid conflicts
const TEST_PORT = 19876;

/**
 * Helper to make HTTP requests
 */
function makeRequest(options: {
  method: string;
  path: string;
  headers?: Record<string, string>;
  body?: string;
}): Promise<{
  statusCode: number;
  headers: http.IncomingHttpHeaders;
  body: string;
}> {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: 'localhost',
        port: TEST_PORT,
        method: options.method,
        path: options.path,
        headers: options.headers || {},
        timeout: 5000,
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
      }
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

describe('HTTP Transport', () => {
  let transport: HTTPTransport;
  let server: TopGunMCPServer;

  beforeEach(async () => {
    server = new TopGunMCPServer();

    transport = new HTTPTransport({ port: TEST_PORT });
    await transport.start(server);

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
        client.close();
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
      expect(data.transport).toBe('http+sse');
      expect(data.mcpPath).toBe('/mcp');
      expect(data.eventPath).toBe('/mcp/events');
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

    it('should handle OPTIONS for CORS preflight', async () => {
      const response = await makeRequest({
        method: 'OPTIONS',
        path: '/mcp',
      });

      expect(response.statusCode).toBe(204);
      expect(response.headers['access-control-allow-origin']).toBeDefined();
      expect(response.headers['access-control-allow-methods']).toBeDefined();
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

  describe('CORS', () => {
    it('should include Access-Control headers', async () => {
      const response = await makeRequest({
        method: 'GET',
        path: '/health',
        headers: {
          Origin: 'http://example.com',
        },
      });

      expect(response.headers['access-control-allow-origin']).toBeDefined();
      expect(response.headers['access-control-allow-methods']).toContain('GET');
      expect(response.headers['access-control-allow-headers']).toContain('Content-Type');
    });

    it('should allow wildcard origin by default', async () => {
      const response = await makeRequest({
        method: 'GET',
        path: '/health',
      });

      expect(response.headers['access-control-allow-origin']).toBe('*');
    });

    it('should respect specific origin restrictions', async () => {
      await transport.stop();
      const client = server.getClient();
      if (client) {
        client.close();
      }

      server = new TopGunMCPServer();
      transport = new HTTPTransport({
        port: TEST_PORT,
        corsOrigins: ['http://allowed.com'],
      });
      await transport.start(server);

      const response = await makeRequest({
        method: 'GET',
        path: '/health',
        headers: {
          Origin: 'http://allowed.com',
        },
      });

      expect(response.headers['access-control-allow-origin']).toBe('http://allowed.com');
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
        })
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
      expect(data.result.content[0].text).toContain('key1');
      expect(data.result.content[0].text).toContain('hello');
    });

    it('should execute list_maps tool via HTTP POST', async () => {
      // Create server with allowed maps
      await transport.stop();
      const client = server.getClient();
      if (client) {
        client.close();
      }

      server = new TopGunMCPServer({
        allowedMaps: ['map1', 'map2'],
      });

      transport = new HTTPTransport({ port: TEST_PORT });
      await transport.start(server);

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
