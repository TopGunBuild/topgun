/**
 * Phase 14D-1: Bootstrap Controller Authentication Tests
 */

import { BootstrapController } from '../bootstrap/BootstrapController';
import { IncomingMessage, ServerResponse } from 'http';
import * as fs from 'fs';
import * as path from 'path';
import * as crypto from 'crypto';
import * as jwt from 'jsonwebtoken';

// Test configuration
const TEST_DATA_DIR = path.join(__dirname, '.test-data-auth');
const TEST_CONFIG_PATH = path.join(TEST_DATA_DIR, 'topgun.json');
const JWT_SECRET = 'test-jwt-secret-for-bootstrap';

// Mock IncomingMessage
function createMockRequest(
  url: string,
  method: string,
  body?: object,
  headers: Record<string, string> = {}
): IncomingMessage {
  const req = {
    url,
    method,
    headers: {
      'content-type': 'application/json',
      ...headers,
    },
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

// Helper to create password hash (matching BootstrapController format)
async function hashPassword(password: string): Promise<string> {
  const salt = crypto.randomBytes(16).toString('hex');
  return new Promise((resolve, reject) => {
    crypto.scrypt(password, salt, 64, (err, derivedKey) => {
      if (err) reject(err);
      else resolve(`${salt}:${derivedKey.toString('hex')}`);
    });
  });
}

describe('BootstrapController Authentication', () => {
  let controller: BootstrapController;

  beforeAll(async () => {
    // Setup test directory and database
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
      server: { port: 8080, metricsPort: 9091 },
      integrations: { mcp: { enabled: false }, vectorSearch: { enabled: false } },
      createdAt: new Date().toISOString(),
    };
    fs.writeFileSync(TEST_CONFIG_PATH, JSON.stringify(config, null, 2));

    // Create SQLite database with admin user
    const Database = (await import('better-sqlite3')).default;
    const db = new Database(path.join(TEST_DATA_DIR, 'topgun.db'));

    db.exec(`
      CREATE TABLE IF NOT EXISTS _users (
        id TEXT PRIMARY KEY,
        username TEXT UNIQUE NOT NULL,
        password_hash TEXT NOT NULL,
        email TEXT,
        role TEXT DEFAULT 'user',
        created_at INTEGER DEFAULT (strftime('%s', 'now'))
      );
    `);

    // Insert test admin user
    const passwordHash = await hashPassword('testpass123');
    db.prepare(`
      INSERT OR REPLACE INTO _users (id, username, password_hash, email, role)
      VALUES (?, ?, ?, ?, 'admin')
    `).run(crypto.randomUUID(), 'testadmin', passwordHash, 'admin@test.com');

    db.close();
  });

  afterAll(() => {
    // Cleanup
    if (fs.existsSync(TEST_DATA_DIR)) {
      fs.rmSync(TEST_DATA_DIR, { recursive: true, force: true });
    }
  });

  beforeEach(() => {
    controller = new BootstrapController({
      configPath: TEST_CONFIG_PATH,
      dataDir: TEST_DATA_DIR,
      jwtSecret: JWT_SECRET,
    });
  });

  describe('POST /api/auth/login', () => {
    it('should return 401 for invalid username', async () => {
      const req = createMockRequest('/api/auth/login', 'POST', {
        username: 'nonexistent',
        password: 'anypass',
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(401);
      const body = JSON.parse(res.body);
      expect(body.error).toBe('Invalid credentials');
    });

    it('should return 401 for invalid password', async () => {
      const req = createMockRequest('/api/auth/login', 'POST', {
        username: 'testadmin',
        password: 'wrongpassword',
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(401);
      const body = JSON.parse(res.body);
      expect(body.error).toBe('Invalid credentials');
    });

    it('should return 400 for missing credentials', async () => {
      const req = createMockRequest('/api/auth/login', 'POST', {
        username: 'testadmin',
        // missing password
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(400);
      const body = JSON.parse(res.body);
      expect(body.error).toBe('Username and password are required');
    });

    it('should return JWT token for valid credentials', async () => {
      const req = createMockRequest('/api/auth/login', 'POST', {
        username: 'testadmin',
        password: 'testpass123',
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const body = JSON.parse(res.body);
      expect(body.token).toBeDefined();
      expect(body.user).toBeDefined();
      expect(body.user.username).toBe('testadmin');
      expect(body.user.role).toBe('admin');

      // Verify JWT structure
      const decoded = jwt.verify(body.token, JWT_SECRET) as jwt.JwtPayload;
      expect(decoded.username).toBe('testadmin');
      expect(decoded.roles).toContain('ADMIN');
      expect(decoded.exp).toBeDefined();
    });
  });

  describe('Admin endpoint protection', () => {
    it('should return 401 for /api/admin/maps without token', async () => {
      const req = createMockRequest('/api/admin/maps', 'GET');
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(401);
      const body = JSON.parse(res.body);
      expect(body.error).toBe('Unauthorized');
    });

    it('should return 401 for /api/admin/cluster/status without token', async () => {
      const req = createMockRequest('/api/admin/cluster/status', 'GET');
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(401);
      const body = JSON.parse(res.body);
      expect(body.error).toBe('Unauthorized');
    });

    it('should return 401 for invalid token', async () => {
      const req = createMockRequest('/api/admin/maps', 'GET', undefined, {
        authorization: 'Bearer invalid.jwt.token',
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(401);
    });

    it('should return 401 for expired token', async () => {
      // Create expired token
      const expiredToken = jwt.sign(
        { userId: 'test', username: 'testadmin', roles: ['ADMIN'] },
        JWT_SECRET,
        { expiresIn: '-1h' }
      );

      const req = createMockRequest('/api/admin/maps', 'GET', undefined, {
        authorization: `Bearer ${expiredToken}`,
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(401);
    });

    it('should allow access to /api/admin/maps with valid token', async () => {
      // First login to get token
      const loginReq = createMockRequest('/api/auth/login', 'POST', {
        username: 'testadmin',
        password: 'testpass123',
      });
      const loginRes = createMockResponse();
      await controller.handle(loginReq, loginRes);

      const { token } = JSON.parse(loginRes.body);

      // Now try admin endpoint with token
      const req = createMockRequest('/api/admin/maps', 'GET', undefined, {
        authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const body = JSON.parse(res.body);
      expect(body.maps).toBeDefined();
      expect(Array.isArray(body.maps)).toBe(true);
    });

    it('should allow access to /api/admin/cluster/status with valid token', async () => {
      // First login to get token
      const loginReq = createMockRequest('/api/auth/login', 'POST', {
        username: 'testadmin',
        password: 'testpass123',
      });
      const loginRes = createMockResponse();
      await controller.handle(loginReq, loginRes);

      const { token } = JSON.parse(loginRes.body);

      // Now try cluster status endpoint with token
      const req = createMockRequest('/api/admin/cluster/status', 'GET', undefined, {
        authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const body = JSON.parse(res.body);
      expect(body.totalPartitions).toBe(271);
      expect(body.nodes).toBeDefined();
      expect(body.partitions).toBeDefined();
      expect(body.isRebalancing).toBeDefined();
    });
  });

  describe('Cluster status response', () => {
    it('should include totalPartitions field', async () => {
      // Login first
      const loginReq = createMockRequest('/api/auth/login', 'POST', {
        username: 'testadmin',
        password: 'testpass123',
      });
      const loginRes = createMockResponse();
      await controller.handle(loginReq, loginRes);
      const { token } = JSON.parse(loginRes.body);

      // Get cluster status
      const req = createMockRequest('/api/admin/cluster/status', 'GET', undefined, {
        authorization: `Bearer ${token}`,
      });
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const body = JSON.parse(res.body);
      expect(body.totalPartitions).toBe(271);
    });
  });

  describe('Public endpoints', () => {
    it('should allow /api/status without authentication', async () => {
      const req = createMockRequest('/api/status', 'GET');
      const res = createMockResponse();

      await controller.handle(req, res);

      expect(res.statusCode).toBe(200);
      const body = JSON.parse(res.body);
      expect(body.configured).toBe(true);
      expect(body.mode).toBe('normal');
    });
  });
});
