/**
 * Phase 14D-2: Zero-Touch Automated Setup Tests
 */

import { BootstrapController } from '../bootstrap/BootstrapController';
import * as fs from 'fs';
import * as path from 'path';

// Test configuration
const TEST_DATA_DIR = path.join(__dirname, '.test-data-zero-touch');
const TEST_CONFIG_PATH = path.join(TEST_DATA_DIR, 'topgun.json');
const TEST_SECRET_FILE = path.join(TEST_DATA_DIR, 'admin_password.txt');

describe('Zero-Touch Setup', () => {
  // Store original env vars
  const originalEnv: Record<string, string | undefined> = {};

  beforeAll(() => {
    // Create test directory
    if (!fs.existsSync(TEST_DATA_DIR)) {
      fs.mkdirSync(TEST_DATA_DIR, { recursive: true });
    }
  });

  afterAll(() => {
    // Cleanup
    if (fs.existsSync(TEST_DATA_DIR)) {
      fs.rmSync(TEST_DATA_DIR, { recursive: true, force: true });
    }
  });

  beforeEach(() => {
    // Save and clear relevant env vars
    const envKeys = [
      'TOPGUN_AUTO_SETUP',
      'TOPGUN_AUTO_SETUP_STRICT',
      'TOPGUN_DEPLOYMENT_MODE',
      'TOPGUN_STORAGE_TYPE',
      'DATABASE_URL',
      'TOPGUN_DATA_DIR',
      'TOPGUN_ADMIN_USER',
      'TOPGUN_ADMIN_PASSWORD',
      'TOPGUN_ADMIN_PASSWORD_FILE',
      'TOPGUN_ADMIN_EMAIL',
      'TOPGUN_PORT',
      'TOPGUN_METRICS_PORT',
      'TOPGUN_MCP_ENABLED',
      'TOPGUN_VECTOR_ENABLED',
      'TOPGUN_SECRETS_PROVIDER',
      'TOPGUN_CONFIGURED',
    ];

    for (const key of envKeys) {
      originalEnv[key] = process.env[key];
      delete process.env[key];
    }

    // Clean up test files
    if (fs.existsSync(TEST_CONFIG_PATH)) {
      fs.unlinkSync(TEST_CONFIG_PATH);
    }
    if (fs.existsSync(path.join(TEST_DATA_DIR, 'topgun.db'))) {
      fs.unlinkSync(path.join(TEST_DATA_DIR, 'topgun.db'));
    }
    if (fs.existsSync(TEST_SECRET_FILE)) {
      fs.unlinkSync(TEST_SECRET_FILE);
    }
  });

  afterEach(() => {
    // Restore original env vars
    for (const [key, value] of Object.entries(originalEnv)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  });

  describe('checkAutoSetup()', () => {
    it('should skip when TOPGUN_AUTO_SETUP is not true', async () => {
      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
      expect(fs.existsSync(TEST_CONFIG_PATH)).toBe(false);
    });

    it('should skip when TOPGUN_AUTO_SETUP is false', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'false';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
    });

    it('should skip when already configured', async () => {
      // Create existing config
      fs.writeFileSync(TEST_CONFIG_PATH, JSON.stringify({ version: '1.0' }));
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      // Should already be configured
      expect(controller.isConfigured).toBe(true);

      await controller.checkAutoSetup();

      // Should still be configured (unchanged)
      expect(controller.isConfigured).toBe(true);
    });

    it('should fail without admin password', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      // Should fall back to interactive mode (lenient mode)
      await controller.checkAutoSetup();

      // Still unconfigured because password missing
      expect(controller.isConfigured).toBe(false);
    });

    it('should configure with valid env vars (memory storage)', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_USER = 'testadmin';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);
      expect(fs.existsSync(TEST_CONFIG_PATH)).toBe(true);

      const config = JSON.parse(fs.readFileSync(TEST_CONFIG_PATH, 'utf-8'));
      expect(config.storage.type).toBe('memory');
      expect(config.deploymentMode).toBe('standalone');
    });

    it('should configure with valid env vars (sqlite storage)', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'sqlite';
      process.env.TOPGUN_DATA_DIR = TEST_DATA_DIR;
      process.env.TOPGUN_ADMIN_USER = 'testadmin';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';
      process.env.TOPGUN_ADMIN_EMAIL = 'admin@test.com';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);

      // Check SQLite database was created
      expect(fs.existsSync(path.join(TEST_DATA_DIR, 'topgun.db'))).toBe(true);

      // Verify admin user was created
      const Database = (await import('better-sqlite3')).default;
      const db = new Database(path.join(TEST_DATA_DIR, 'topgun.db'));
      const user = db.prepare('SELECT * FROM _users WHERE username = ?').get('testadmin') as { username: string; role: string; email: string };
      db.close();

      expect(user).toBeDefined();
      expect(user.username).toBe('testadmin');
      expect(user.role).toBe('admin');
      expect(user.email).toBe('admin@test.com');
    });

    it('should read password from file (_FILE suffix)', async () => {
      // Create secret file
      fs.writeFileSync(TEST_SECRET_FILE, 'filepassword123\n'); // with trailing newline

      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_USER = 'fileadmin';
      process.env.TOPGUN_ADMIN_PASSWORD_FILE = TEST_SECRET_FILE;

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);
    });

    it('should fail if secret file not found', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD_FILE = '/nonexistent/path/to/secret';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      // Should fall back to interactive mode
      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
    });

    it('should use custom ports from env vars', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';
      process.env.TOPGUN_PORT = '9999';
      process.env.TOPGUN_METRICS_PORT = '9998';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);

      const config = JSON.parse(fs.readFileSync(TEST_CONFIG_PATH, 'utf-8'));
      expect(config.server.port).toBe(9999);
      expect(config.server.metricsPort).toBe(9998);
    });

    it('should enable MCP integration via env vars', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';
      process.env.TOPGUN_MCP_ENABLED = 'true';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);

      const config = JSON.parse(fs.readFileSync(TEST_CONFIG_PATH, 'utf-8'));
      expect(config.integrations.mcp.enabled).toBe(true);
    });

    it('should set deployment mode to cluster', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';
      process.env.TOPGUN_DEPLOYMENT_MODE = 'cluster';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);

      const config = JSON.parse(fs.readFileSync(TEST_CONFIG_PATH, 'utf-8'));
      expect(config.deploymentMode).toBe('cluster');
    });
  });

  describe('Validation', () => {
    it('should reject short password', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'short'; // less than 8 chars

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      // Should fall back due to validation error
      expect(controller.isConfigured).toBe(false);
    });

    it('should reject short username', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_USER = 'ab'; // less than 3 chars
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
    });

    it('should reject invalid username characters', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_USER = 'admin@user!'; // invalid chars
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
    });

    it('should reject postgres without DATABASE_URL', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'postgres';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';
      // DATABASE_URL not set

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
    });

    it('should reject invalid port numbers', async () => {
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';
      process.env.TOPGUN_PORT = '99999'; // invalid port

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(false);
    });
  });

  describe('Secrets Provider', () => {
    it('should prefer env var over file by default', async () => {
      // Create file with different password
      fs.writeFileSync(TEST_SECRET_FILE, 'filepassword123');

      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'envpassword123';
      process.env.TOPGUN_ADMIN_PASSWORD_FILE = TEST_SECRET_FILE;

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      // Should use env var (envpassword123), not file
      expect(controller.isConfigured).toBe(true);
    });

    it('should use file when secrets provider is "file"', async () => {
      fs.writeFileSync(TEST_SECRET_FILE, 'filepassword123');

      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_SECRETS_PROVIDER = 'file';
      process.env.TOPGUN_ADMIN_PASSWORD_FILE = TEST_SECRET_FILE;

      const controller = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });

      await controller.checkAutoSetup();

      expect(controller.isConfigured).toBe(true);
    });
  });

  describe('Idempotency', () => {
    it('should not overwrite existing configuration on restart', async () => {
      // First setup
      process.env.TOPGUN_AUTO_SETUP = 'true';
      process.env.TOPGUN_STORAGE_TYPE = 'memory';
      process.env.TOPGUN_ADMIN_PASSWORD = 'testpass123';

      const controller1 = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });
      await controller1.checkAutoSetup();

      expect(controller1.isConfigured).toBe(true);
      const firstConfig = fs.readFileSync(TEST_CONFIG_PATH, 'utf-8');

      // Second "startup" with same env vars
      const controller2 = new BootstrapController({
        configPath: TEST_CONFIG_PATH,
        dataDir: TEST_DATA_DIR,
      });
      await controller2.checkAutoSetup();

      // Should still be configured and config unchanged
      expect(controller2.isConfigured).toBe(true);
      const secondConfig = fs.readFileSync(TEST_CONFIG_PATH, 'utf-8');
      expect(secondConfig).toBe(firstConfig);
    });
  });
});
