/**
 * Phase 14D-3: Settings Controller Tests
 */

import { SettingsController } from '../settings/SettingsController';
import { IncomingMessage, ServerResponse } from 'http';
import * as fs from 'fs';
import * as path from 'path';
import * as jwt from 'jsonwebtoken';

// Test configuration
const TEST_DATA_DIR = path.join(__dirname, '.test-data-settings');
const TEST_CONFIG_PATH = path.join(TEST_DATA_DIR, 'topgun.json');
const TEST_RUNTIME_PATH = path.join(TEST_DATA_DIR, 'topgun.runtime.json');
const JWT_SECRET = 'test-jwt-secret-for-settings';

// Mock IncomingMessage
function createMockRequest(
  url: string,
  method: string,
  body?: object,
  headers: Record<string, string> = {}
): IncomingMessage {
  // Normalize headers to lowercase (HTTP spec requires case-insensitive headers)
  const normalizedHeaders: Record<string, string> = {
    'content-type': 'application/json',
  };
  for (const [key, value] of Object.entries(headers)) {
    normalizedHeaders[key.toLowerCase()] = value;
  }

  const req = {
    url,
    method,
    headers: normalizedHeaders,
    on: jest.fn((event, callback) => {
      if (event === 'data' && body) {
        callback(JSON.stringify(body));
      }
      if (event === 'end') {
        callback();
      }
    }),
  } as unknown as IncomingMessage;
  return req;
}

// Mock ServerResponse
function createMockResponse(): ServerResponse & {
  statusCode: number;
  body: string;
  headers: Record<string, string>;
} {
  const headers: Record<string, string> = {};
  let body = '';
  const res = {
    statusCode: 200,
    body: '',
    headers,
    setHeader: jest.fn((key: string, value: string) => {
      headers[key] = value;
    }),
    end: jest.fn((data?: string) => {
      body = data || '';
      res.body = body;
    }),
  } as unknown as ServerResponse & {
    statusCode: number;
    body: string;
    headers: Record<string, string>;
  };
  return res;
}

// Generate valid JWT token
function generateToken(payload: object = {}): string {
  return jwt.sign(
    {
      sub: 'test-user',
      userId: 'test-user',
      username: 'admin',
      roles: ['ADMIN'],
      ...payload,
    },
    JWT_SECRET,
    { algorithm: 'HS256', expiresIn: '1h' }
  );
}

describe('SettingsController', () => {
  let controller: SettingsController;

  beforeAll(() => {
    // Setup test directory
    if (!fs.existsSync(TEST_DATA_DIR)) {
      fs.mkdirSync(TEST_DATA_DIR, { recursive: true });
    }

    // Create test config
    const config = {
      version: '1.0',
      deploymentMode: 'standalone',
      storage: {
        type: 'sqlite',
        dataDir: TEST_DATA_DIR,
      },
      server: { port: 8080, metricsPort: 9090 },
      integrations: {
        mcp: { enabled: false, port: 3001 },
        vectorSearch: { enabled: false },
      },
    };
    fs.writeFileSync(TEST_CONFIG_PATH, JSON.stringify(config, null, 2));
  });

  beforeEach(() => {
    // Remove runtime settings between tests
    if (fs.existsSync(TEST_RUNTIME_PATH)) {
      fs.unlinkSync(TEST_RUNTIME_PATH);
    }

    controller = new SettingsController({
      configPath: TEST_CONFIG_PATH,
      jwtSecret: JWT_SECRET,
    });
  });

  afterAll(() => {
    // Cleanup
    if (fs.existsSync(TEST_DATA_DIR)) {
      fs.rmSync(TEST_DATA_DIR, { recursive: true, force: true });
    }
  });

  describe('Authentication', () => {
    it('should reject request without Authorization header', async () => {
      const req = createMockRequest('/api/admin/settings', 'GET');
      const res = createMockResponse();

      const handled = await controller.handle(req, res);

      expect(handled).toBe(true);
      expect(res.statusCode).toBe(401);
      expect(JSON.parse(res.body)).toEqual({ error: 'Unauthorized' });
    });

    it('should reject request with invalid token', async () => {
      const req = createMockRequest('/api/admin/settings', 'GET', undefined, {
        Authorization: 'Bearer invalid-token',
      });
      const res = createMockResponse();

      const handled = await controller.handle(req, res);

      expect(handled).toBe(true);
      expect(res.statusCode).toBe(401);
    });

    it('should accept request with valid token', async () => {
      const token = generateToken();
      const req = createMockRequest('/api/admin/settings', 'GET', undefined, {
        Authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      const handled = await controller.handle(req, res);

      expect(handled).toBe(true);
      expect(res.statusCode).toBe(200);
    });
  });

  describe('GET /api/admin/settings', () => {
    it('should return current settings with masked secrets', async () => {
      const token = generateToken();
      const req = createMockRequest('/api/admin/settings', 'GET', undefined, {
        Authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);

      // Check structure
      expect(data).toHaveProperty('general');
      expect(data).toHaveProperty('storage');
      expect(data).toHaveProperty('security');
      expect(data).toHaveProperty('integrations');
      expect(data).toHaveProperty('cluster');
      expect(data).toHaveProperty('rateLimits');
      expect(data).toHaveProperty('_meta');

      // Check defaults
      expect(data.general.logLevel).toBe('info');
      expect(data.rateLimits.connections).toBe(1000);
      expect(data.rateLimits.messagesPerSecond).toBe(10000);

      // Check meta
      expect(data._meta.hotReloadable).toContain('logLevel');
      expect(data._meta.restartRequired).toContain('port');
    });

    it('should mask connection string password', async () => {
      // Create config with connection string
      const configWithDb = {
        storage: {
          type: 'postgres',
          connectionString: 'postgresql://user:secret123@localhost:5432/db',
        },
      };
      fs.writeFileSync(TEST_CONFIG_PATH, JSON.stringify(configWithDb, null, 2));

      const newController = new SettingsController({
        configPath: TEST_CONFIG_PATH,
        jwtSecret: JWT_SECRET,
      });

      const token = generateToken();
      const req = createMockRequest('/api/admin/settings', 'GET', undefined, {
        Authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      await newController.handle(req, res);

      const data = JSON.parse(res.body);
      expect(data.storage.connectionString).toContain('***');
      expect(data.storage.connectionString).not.toContain('secret123');
    });
  });

  describe('PATCH /api/admin/settings', () => {
    it('should update hot-reloadable settings', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings',
        'PATCH',
        { logLevel: 'debug' },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.success).toBe(true);
      expect(data.updated).toContain('logLevel');
      expect(data.rejected).toEqual([]);

      // Verify change persisted
      const settings = controller.getRuntimeSettings();
      expect(settings.logLevel).toBe('debug');
    });

    it('should reject restart-required settings', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings',
        'PATCH',
        { port: 9000 },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(400);
      const data = JSON.parse(res.body);
      expect(data.success).toBe(false);
      expect(data.rejected).toContain('port');
    });

    it('should update nested settings', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings',
        'PATCH',
        {
          rateLimits: {
            connections: 500,
            messagesPerSecond: 5000,
          },
        },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.success).toBe(true);
      expect(data.updated).toContain('rateLimits.connections');
      expect(data.updated).toContain('rateLimits.messagesPerSecond');

      const settings = controller.getRuntimeSettings();
      expect(settings.rateLimits.connections).toBe(500);
      expect(settings.rateLimits.messagesPerSecond).toBe(5000);
    });

    it('should update integration toggles', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings',
        'PATCH',
        {
          integrations: {
            mcp: { enabled: true },
            vectorSearch: { enabled: true },
          },
        },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.success).toBe(true);

      const settings = controller.getRuntimeSettings();
      expect(settings.integrations.mcp.enabled).toBe(true);
      expect(settings.integrations.vectorSearch.enabled).toBe(true);
    });

    it('should call onSettingsChange callback', async () => {
      const onSettingsChange = jest.fn();
      const controllerWithCallback = new SettingsController({
        configPath: TEST_CONFIG_PATH,
        jwtSecret: JWT_SECRET,
        onSettingsChange,
      });

      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings',
        'PATCH',
        { logLevel: 'warn' },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controllerWithCallback.handle(req, res);

      expect(onSettingsChange).toHaveBeenCalledWith(
        expect.objectContaining({ logLevel: 'warn' })
      );
    });
  });

  describe('POST /api/admin/settings/validate', () => {
    it('should validate log level', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings/validate',
        'POST',
        { logLevel: 'invalid' },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.valid).toBe(false);
      expect(data.errors[0].path).toBe('logLevel');
    });

    it('should validate rate limits (negative)', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings/validate',
        'POST',
        { rateLimits: { messagesPerSecond: -1 } },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.valid).toBe(false);
      expect(data.errors[0].path).toBe('rateLimits.messagesPerSecond');
    });

    it('should validate rate limits (zero is invalid)', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings/validate',
        'POST',
        { rateLimits: { connections: 0 } },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.valid).toBe(false);
      expect(data.errors[0].path).toBe('rateLimits.connections');
      expect(data.errors[0].message).toContain('minimum 1');
    });

    it('should validate boolean fields', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings/validate',
        'POST',
        { metricsEnabled: 'not-a-boolean' },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.valid).toBe(false);
    });

    it('should pass valid settings', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings/validate',
        'POST',
        {
          logLevel: 'debug',
          rateLimits: { connections: 500 },
        },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const data = JSON.parse(res.body);
      expect(data.valid).toBe(true);
      expect(data.errors).toEqual([]);
    });
  });

  describe('Persistence', () => {
    it('should persist runtime settings to file', async () => {
      const token = generateToken();
      const req = createMockRequest(
        '/api/admin/settings',
        'PATCH',
        { logLevel: 'error' },
        { Authorization: `Bearer ${token}` }
      );
      const res = createMockResponse();

      await controller.handle(req, res);

      // Check file was created
      expect(fs.existsSync(TEST_RUNTIME_PATH)).toBe(true);

      const saved = JSON.parse(fs.readFileSync(TEST_RUNTIME_PATH, 'utf-8'));
      expect(saved.logLevel).toBe('error');
    });

    it('should load persisted settings on restart', async () => {
      // Write runtime settings
      fs.writeFileSync(
        TEST_RUNTIME_PATH,
        JSON.stringify({
          logLevel: 'warn',
          rateLimits: { connections: 999, messagesPerSecond: 888 },
        })
      );

      // Create new controller (simulates restart)
      const newController = new SettingsController({
        configPath: TEST_CONFIG_PATH,
        jwtSecret: JWT_SECRET,
      });

      const settings = newController.getRuntimeSettings();
      expect(settings.logLevel).toBe('warn');
      expect(settings.rateLimits.connections).toBe(999);
      expect(settings.rateLimits.messagesPerSecond).toBe(888);
    });
  });

  describe('CORS and OPTIONS', () => {
    it('should handle OPTIONS preflight request', async () => {
      const req = createMockRequest('/api/admin/settings', 'OPTIONS');
      const res = createMockResponse();

      const handled = await controller.handle(req, res);

      expect(handled).toBe(true);
      expect(res.statusCode).toBe(204);
    });
  });

  describe('Unknown routes', () => {
    it('should return false for unknown routes', async () => {
      const token = generateToken();
      const req = createMockRequest('/api/admin/unknown', 'GET', undefined, {
        Authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      const handled = await controller.handle(req, res);

      expect(handled).toBe(false);
    });
  });
});
