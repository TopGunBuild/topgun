/**
 * Phase 14D: Bootstrap Controller
 *
 * Handles initial server setup when no configuration exists.
 * Provides /api/setup endpoints for the Setup Wizard.
 */

import { IncomingMessage, ServerResponse } from 'http';
import * as fs from 'fs';
import * as path from 'path';
import * as crypto from 'crypto';
import * as jwt from 'jsonwebtoken';
import { PARTITION_COUNT } from '@topgunbuild/core';
import { logger } from '../utils/logger';

/**
 * Environment variable names for Zero-Touch Setup
 */
const ENV_KEYS = {
  AUTO_SETUP: 'TOPGUN_AUTO_SETUP',
  AUTO_SETUP_STRICT: 'TOPGUN_AUTO_SETUP_STRICT',
  DEPLOYMENT_MODE: 'TOPGUN_DEPLOYMENT_MODE',
  STORAGE_TYPE: 'TOPGUN_STORAGE_TYPE',
  DATABASE_URL: 'DATABASE_URL',
  DATA_DIR: 'TOPGUN_DATA_DIR',
  ADMIN_USER: 'TOPGUN_ADMIN_USER',
  ADMIN_PASSWORD: 'TOPGUN_ADMIN_PASSWORD',
  ADMIN_PASSWORD_FILE: 'TOPGUN_ADMIN_PASSWORD_FILE',
  ADMIN_EMAIL: 'TOPGUN_ADMIN_EMAIL',
  PORT: 'TOPGUN_PORT',
  METRICS_PORT: 'TOPGUN_METRICS_PORT',
  MCP_ENABLED: 'TOPGUN_MCP_ENABLED',
  MCP_PORT: 'TOPGUN_MCP_PORT',
  MCP_TOKEN: 'TOPGUN_MCP_TOKEN',
  VECTOR_ENABLED: 'TOPGUN_VECTOR_ENABLED',
  VECTOR_MODEL: 'TOPGUN_VECTOR_MODEL',
  SECRETS_PROVIDER: 'TOPGUN_SECRETS_PROVIDER',
} as const;

export interface SetupConfig {
  deploymentMode: 'standalone' | 'cluster';
  storage: {
    type: 'sqlite' | 'postgres' | 'memory';
    connectionString?: string;
    dataDir?: string;
  };
  admin: {
    username: string;
    password: string;
    email?: string;
  };
  server: {
    port: number;
    metricsPort: number;
  };
  integrations: {
    mcpEnabled: boolean;
    mcpPort?: number;
    mcpToken?: string;
    vectorSearchEnabled?: boolean;
    vectorModel?: string;
  };
}

export interface BootstrapStatus {
  configured: boolean;
  version: string;
  mode: 'bootstrap' | 'normal';
}

export interface MapInfo {
  name: string;
  entryCount: number;
}

export interface ClusterNodeInfo {
  id: string;
  address: string;
  status: 'healthy' | 'suspect' | 'dead';
  partitions: number[];
  connections: number;
  memory: { used: number; total: number };
  uptime: number;
}

export interface ClusterStatus {
  nodes: ClusterNodeInfo[];
  partitions: { id: number; owner: string; replicas: string[] }[];
  isRebalancing: boolean;
}

export interface BootstrapControllerConfig {
  configPath?: string;
  dataDir?: string;
  version?: string;
  /** JWT secret for auth tokens */
  jwtSecret?: string;
  /** Function to get list of maps from ServerCoordinator */
  getMaps?: () => Map<string, unknown>;
  /** Function to get cluster status */
  getClusterStatus?: () => ClusterStatus;
}

export class BootstrapController {
  private configPath: string;
  private dataDir: string;
  private version: string;
  private jwtSecret: string;
  private _isConfigured: boolean;
  private getMaps?: () => Map<string, unknown>;
  private getClusterStatus?: () => ClusterStatus;

  constructor(config: BootstrapControllerConfig = {}) {
    this.configPath = config.configPath || process.env.TOPGUN_CONFIG_PATH || 'topgun.json';
    this.dataDir = config.dataDir || process.env.TOPGUN_DATA_DIR || './data';
    this.version = config.version || process.env.npm_package_version || '0.0.0';
    this.jwtSecret = config.jwtSecret || process.env.JWT_SECRET || 'topgun-secret-dev';
    this._isConfigured = this.checkConfiguration();
    this.getMaps = config.getMaps;
    this.getClusterStatus = config.getClusterStatus;
  }

  /**
   * Set data accessor functions after construction (for deferred initialization)
   */
  setDataAccessors(accessors: {
    getMaps?: () => Map<string, unknown>;
    getClusterStatus?: () => ClusterStatus;
  }): void {
    if (accessors.getMaps) this.getMaps = accessors.getMaps;
    if (accessors.getClusterStatus) this.getClusterStatus = accessors.getClusterStatus;
  }

  /**
   * Check if server is already configured
   */
  private checkConfiguration(): boolean {
    // Check for config file
    if (fs.existsSync(this.configPath)) {
      return true;
    }

    // Check for essential environment variables (alternative to config file)
    if (process.env.TOPGUN_CONFIGURED === 'true') {
      return true;
    }

    // Check for DATABASE_URL (indicates production setup)
    if (process.env.DATABASE_URL) {
      return true;
    }

    return false;
  }

  /**
   * Returns true if server needs initial setup
   */
  get isBootstrapMode(): boolean {
    return !this._isConfigured;
  }

  /**
   * Returns true if server is configured
   */
  get isConfigured(): boolean {
    return this._isConfigured;
  }

  // ============================================
  // Zero-Touch Setup (Phase 14D-2)
  // ============================================

  /**
   * Check for auto-setup environment and run if enabled.
   * Called during server initialization before starting HTTP servers.
   */
  async checkAutoSetup(): Promise<void> {
    if (process.env[ENV_KEYS.AUTO_SETUP] !== 'true') {
      return;
    }

    if (this._isConfigured) {
      logger.info('[AutoSetup] Server already configured, skipping auto-setup');
      return;
    }

    logger.info('[AutoSetup] Starting automatic configuration...');

    try {
      const config = this.buildConfigFromEnv();
      this.validateAutoSetupConfig(config);
      await this.initializeStorage(config.storage);
      await this.createAdminUser(config.admin, config.storage);
      await this.saveConfiguration(config);

      this._isConfigured = true;
      logger.info('[AutoSetup] Configuration complete');
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unknown error';
      logger.error({ error }, `[AutoSetup] Failed: ${message}`);

      // Decide behavior based on strict mode
      if (process.env[ENV_KEYS.AUTO_SETUP_STRICT] === 'true') {
        // Strict mode: fail fast
        logger.error('[AutoSetup] Strict mode enabled, exiting...');
        process.exit(1);
      } else {
        // Lenient mode: continue to bootstrap mode (show Setup Wizard)
        logger.warn('[AutoSetup] Falling back to interactive setup mode');
      }
    }
  }

  /**
   * Build SetupConfig from environment variables
   */
  private buildConfigFromEnv(): SetupConfig {
    const password = this.resolveSecret(ENV_KEYS.ADMIN_PASSWORD);

    if (!password) {
      throw new Error(
        `${ENV_KEYS.ADMIN_PASSWORD} or ${ENV_KEYS.ADMIN_PASSWORD_FILE} is required for auto-setup`
      );
    }

    const storageType = (process.env[ENV_KEYS.STORAGE_TYPE] as 'sqlite' | 'postgres' | 'memory') || 'sqlite';

    return {
      deploymentMode: (process.env[ENV_KEYS.DEPLOYMENT_MODE] as 'standalone' | 'cluster') || 'standalone',
      storage: {
        type: storageType,
        connectionString: process.env[ENV_KEYS.DATABASE_URL],
        dataDir: process.env[ENV_KEYS.DATA_DIR] || './data',
      },
      admin: {
        username: process.env[ENV_KEYS.ADMIN_USER] || 'admin',
        password,
        email: process.env[ENV_KEYS.ADMIN_EMAIL],
      },
      server: {
        port: parseInt(process.env[ENV_KEYS.PORT] || '8080', 10),
        metricsPort: parseInt(process.env[ENV_KEYS.METRICS_PORT] || '9090', 10),
      },
      integrations: {
        mcpEnabled: process.env[ENV_KEYS.MCP_ENABLED] === 'true',
        mcpPort: process.env[ENV_KEYS.MCP_PORT] ? parseInt(process.env[ENV_KEYS.MCP_PORT]!, 10) : undefined,
        mcpToken: process.env[ENV_KEYS.MCP_TOKEN],
        vectorSearchEnabled: process.env[ENV_KEYS.VECTOR_ENABLED] === 'true',
        vectorModel: process.env[ENV_KEYS.VECTOR_MODEL],
      },
    };
  }

  /**
   * Resolve secret from environment variable or file.
   * Supports Docker Secrets and Kubernetes Secrets mounted as files.
   */
  private resolveSecret(key: string): string | undefined {
    const provider = process.env[ENV_KEYS.SECRETS_PROVIDER] || 'env';

    switch (provider) {
      case 'file':
        return this.resolveSecretFromFile(key);
      case 'env':
      default:
        // Try direct env var first, then fall back to file
        return process.env[key] || this.resolveSecretFromFile(key);
    }
  }

  /**
   * Read secret from file (Docker Secrets, Kubernetes Secrets mounted as files).
   * Looks for {KEY}_FILE environment variable pointing to the secret file.
   *
   * Note: Uses synchronous file reading intentionally - this runs during server
   * startup before any async operations begin. Sync read is appropriate here
   * as it blocks only the initialization phase, not request handling.
   */
  private resolveSecretFromFile(key: string): string | undefined {
    const fileKey = `${key}_FILE`;
    const filePath = process.env[fileKey];

    if (filePath) {
      if (!fs.existsSync(filePath)) {
        throw new Error(`Secret file not found: ${filePath}`);
      }
      return fs.readFileSync(filePath, 'utf-8').trim();
    }

    return undefined;
  }

  /**
   * Validate auto-setup configuration with detailed error messages
   */
  private validateAutoSetupConfig(config: SetupConfig): void {
    // Password requirements
    if (!config.admin.password) {
      throw new Error('Admin password is required');
    }
    if (config.admin.password.length < 8) {
      throw new Error('Admin password must be at least 8 characters');
    }

    // Storage validation
    if (config.storage.type === 'postgres' && !config.storage.connectionString) {
      throw new Error(`${ENV_KEYS.DATABASE_URL} is required for PostgreSQL storage`);
    }

    // Port validation
    if (config.server.port < 1 || config.server.port > 65535) {
      throw new Error(`Invalid port number: must be between 1 and 65535`);
    }
    if (config.server.metricsPort < 1 || config.server.metricsPort > 65535) {
      throw new Error(`Invalid metrics port number: must be between 1 and 65535`);
    }

    // Username validation
    if (config.admin.username.length < 3) {
      throw new Error('Admin username must be at least 3 characters');
    }
    if (!/^[a-zA-Z0-9_-]+$/.test(config.admin.username)) {
      throw new Error('Admin username can only contain alphanumeric characters, underscores, and hyphens');
    }
  }

  // ============================================
  // HTTP Request Handlers
  // ============================================

  /**
   * Handle HTTP requests for bootstrap endpoints
   */
  async handle(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    const url = req.url || '';
    const method = req.method || 'GET';

    // Set CORS headers for admin dashboard
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');

    // Handle preflight
    if (method === 'OPTIONS') {
      res.statusCode = 204;
      res.end();
      return true;
    }

    // Status endpoint - always available
    if (url === '/api/status' && method === 'GET') {
      return this.handleStatus(res);
    }

    // Login endpoint - available when configured
    if (url === '/api/auth/login' && method === 'POST' && this.isConfigured) {
      return this.handleLogin(req, res);
    }

    // Bootstrap-only endpoints
    if (this.isBootstrapMode) {
      if (url === '/api/setup/test-connection' && method === 'POST') {
        return this.handleTestConnection(req, res);
      }

      if (url === '/api/setup' && method === 'POST') {
        return this.handleSetup(req, res);
      }
    }

    // Admin endpoints (available when configured, require auth)
    if (this.isConfigured) {
      // Verify JWT for all admin endpoints
      if (url.startsWith('/api/admin/')) {
        const authResult = this.verifyAdminToken(req);
        if (!authResult.valid) {
          this.sendJson(res, 401, { error: 'Unauthorized' });
          return true;
        }
      }

      if (url === '/api/admin/maps' && method === 'GET') {
        return this.handleListMaps(res);
      }

      if (url === '/api/admin/cluster/status' && method === 'GET') {
        return this.handleClusterStatus(res);
      }
    }

    return false;
  }

  /**
   * GET /api/status
   */
  private handleStatus(res: ServerResponse): boolean {
    const status: BootstrapStatus = {
      configured: this._isConfigured,
      version: this.version,
      mode: this.isBootstrapMode ? 'bootstrap' : 'normal',
    };

    res.setHeader('Content-Type', 'application/json');
    res.statusCode = 200;
    res.end(JSON.stringify(status));
    return true;
  }

  /**
   * POST /api/setup/test-connection
   */
  private async handleTestConnection(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const { type, connectionString, dataDir } = body;

      if (type === 'postgres') {
        const pg = await import('pg');
        const client = new pg.default.Client({ connectionString });
        await client.connect();
        await client.query('SELECT 1');
        await client.end();
        this.sendJson(res, 200, { success: true, message: 'Connection successful' });
      } else if (type === 'sqlite') {
        const Database = (await import('better-sqlite3')).default;
        const dir = dataDir || this.dataDir;

        // Ensure directory exists
        if (!fs.existsSync(dir)) {
          fs.mkdirSync(dir, { recursive: true });
        }

        const testPath = path.join(dir, 'test.db');
        const db = new Database(testPath);
        db.exec('SELECT 1');
        db.close();

        // Clean up test file
        if (fs.existsSync(testPath)) {
          fs.unlinkSync(testPath);
        }

        this.sendJson(res, 200, { success: true, message: 'SQLite working' });
      } else {
        this.sendJson(res, 200, { success: true, message: 'Memory mode - no test needed' });
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : 'Unknown error';
      logger.error({ error }, 'Connection test failed');
      this.sendJson(res, 400, { success: false, message });
    }

    return true;
  }

  /**
   * POST /api/setup
   */
  private async handleSetup(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    try {
      const config: SetupConfig = await this.parseBody(req);

      // 1. Validate config
      this.validateConfig(config);

      // 2. Initialize storage
      await this.initializeStorage(config.storage);

      // 3. Create admin user
      const adminUserId = await this.createAdminUser(config.admin, config.storage);

      // 4. Save configuration
      await this.saveConfiguration(config);

      // Mark as configured
      this._isConfigured = true;

      // Generate token for the admin user so they can use it immediately
      const token = this.generateJWT({
        userId: adminUserId,
        username: config.admin.username,
        role: 'ADMIN',
      });

      this.sendJson(res, 200, {
        success: true,
        message: 'Setup complete',
        restartRequired: true,
        token,
      });

      // Schedule restart
      logger.info('[Bootstrap] Setup complete, server will restart in 2 seconds...');
      setTimeout(() => {
        process.exit(0); // Let process manager restart
      }, 2000);

    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : 'Unknown error';
      logger.error({ error }, 'Setup failed');
      this.sendJson(res, 400, { success: false, message });
    }

    return true;
  }

  /**
   * GET /api/admin/maps
   */
  private handleListMaps(res: ServerResponse): boolean {
    const mapInfos: MapInfo[] = [];

    if (this.getMaps) {
      const maps = this.getMaps();
      for (const [name, map] of maps) {
        // Skip system maps from the list
        if (name.startsWith('$sys/')) continue;

        // Get entry count from map
        let entryCount = 0;
        if (map && typeof map === 'object' && 'size' in map) {
          entryCount = (map as Map<unknown, unknown>).size;
        } else if (map && typeof map === 'object' && 'entries' in map && typeof (map as { entries: () => { size: number } }).entries === 'function') {
          const entries = (map as { entries: () => Map<unknown, unknown> }).entries();
          entryCount = entries.size;
        }

        mapInfos.push({
          name,
          entryCount,
        });
      }
    }

    this.sendJson(res, 200, { maps: mapInfos });
    return true;
  }

  /**
   * GET /api/admin/cluster/status
   */
  private handleClusterStatus(res: ServerResponse): boolean {
    if (this.getClusterStatus) {
      const status = this.getClusterStatus();
      // Transform to match frontend expected format (nodeId instead of id)
      const transformedStatus = {
        ...status,
        totalPartitions: PARTITION_COUNT, // Fixed partition count (see PHASE_14D_AUTH_SECURITY.md)
        nodes: status.nodes.map((node) => ({
          nodeId: node.id,
          address: node.address,
          status: node.status,
          partitionCount: node.partitions.length,
          connections: node.connections,
          memory: node.memory,
          uptime: node.uptime,
        })),
      };
      this.sendJson(res, 200, transformedStatus);
    } else {
      // Return default standalone status
      this.sendJson(res, 200, {
        nodes: [],
        partitions: [],
        totalPartitions: PARTITION_COUNT,
        isRebalancing: false,
      });
    }
    return true;
  }

  /**
   * POST /api/auth/login
   */
  private async handleLogin(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    try {
      const { username, password } = await this.parseBody(req);

      if (!username || !password) {
        this.sendJson(res, 400, { error: 'Username and password are required' });
        return true;
      }

      // Find user in database
      const user = await this.findUser(username);
      if (!user) {
        this.sendJson(res, 401, { error: 'Invalid credentials' });
        return true;
      }

      // Verify password
      const isValid = await this.verifyPassword(password, user.password_hash);
      if (!isValid) {
        this.sendJson(res, 401, { error: 'Invalid credentials' });
        return true;
      }

      // Generate JWT
      const token = this.generateJWT({
        userId: user.id,
        username: user.username,
        role: user.role,
      });

      this.sendJson(res, 200, {
        token,
        user: {
          id: user.id,
          username: user.username,
          role: user.role,
        },
      });
    } catch (error) {
      logger.error({ error }, 'Login failed');
      this.sendJson(res, 500, { error: 'Login failed' });
    }
    return true;
  }

  /**
   * Verify JWT token from Authorization header
   */
  private verifyAdminToken(req: IncomingMessage): { valid: boolean; principal?: Record<string, unknown> } {
    const authHeader = req.headers['authorization'];
    if (!authHeader?.startsWith('Bearer ')) {
      return { valid: false };
    }

    const token = authHeader.slice(7);
    try {
      const decoded = jwt.verify(token, this.jwtSecret);
      return { valid: true, principal: decoded as Record<string, unknown> };
    } catch {
      return { valid: false };
    }
  }

  /**
   * Verify password against stored hash
   */
  private async verifyPassword(password: string, hash: string): Promise<boolean> {
    const [salt, storedHash] = hash.split(':');
    if (!salt || !storedHash) {
      return false;
    }
    return new Promise((resolve, reject) => {
      crypto.scrypt(password, salt, 64, (err, derivedKey) => {
        if (err) reject(err);
        else resolve(derivedKey.toString('hex') === storedHash);
      });
    });
  }

  /**
   * Generate JWT token
   */
  private generateJWT(payload: { userId: string; username: string; role: string }): string {
    return jwt.sign(
      {
        sub: payload.userId,
        userId: payload.userId,
        username: payload.username,
        roles: [payload.role.toUpperCase()],
        iat: Math.floor(Date.now() / 1000),
        exp: Math.floor(Date.now() / 1000) + 24 * 60 * 60, // 24 hours
      },
      this.jwtSecret,
      { algorithm: 'HS256' }
    );
  }

  /**
   * Find user by username in database
   */
  private async findUser(username: string): Promise<{
    id: string;
    username: string;
    password_hash: string;
    role: string;
  } | null> {
    const config = this.loadConfig();

    if (config?.storage?.type === 'postgres') {
      const pg = await import('pg');
      const client = new pg.default.Client({
        connectionString: config.storage.connectionString,
      });
      await client.connect();

      const result = await client.query(
        'SELECT id, username, password_hash, role FROM _users WHERE username = $1',
        [username]
      );
      await client.end();

      return result.rows[0] || null;
    } else if (config?.storage?.type === 'sqlite') {
      const Database = (await import('better-sqlite3')).default;
      const db = new Database(path.join(config.storage.dataDir || this.dataDir, 'topgun.db'));

      const user = db
        .prepare('SELECT id, username, password_hash, role FROM _users WHERE username = ?')
        .get(username) as { id: string; username: string; password_hash: string; role: string } | undefined;
      db.close();

      return user || null;
    }

    return null;
  }

  /**
   * Load configuration from file
   */
  private loadConfig(): {
    storage?: { type: string; connectionString?: string; dataDir?: string };
  } | null {
    if (fs.existsSync(this.configPath)) {
      return JSON.parse(fs.readFileSync(this.configPath, 'utf-8'));
    }
    return null;
  }

  /**
   * Validate setup configuration
   */
  private validateConfig(config: SetupConfig): void {
    if (!config.admin?.username || !config.admin?.password) {
      throw new Error('Admin username and password are required');
    }

    if (config.admin.password.length < 8) {
      throw new Error('Password must be at least 8 characters');
    }

    if (config.storage?.type === 'postgres' && !config.storage.connectionString) {
      throw new Error('PostgreSQL connection string is required');
    }
  }

  /**
   * Initialize storage backend
   */
  private async initializeStorage(storage: SetupConfig['storage']): Promise<void> {
    if (storage.type === 'postgres') {
      const pg = await import('pg');
      const client = new pg.default.Client({ connectionString: storage.connectionString });
      await client.connect();

      // Create system tables
      await client.query(`
        CREATE TABLE IF NOT EXISTS _system (
          key TEXT PRIMARY KEY,
          value JSONB,
          created_at TIMESTAMPTZ DEFAULT NOW(),
          updated_at TIMESTAMPTZ DEFAULT NOW()
        );

        CREATE TABLE IF NOT EXISTS _users (
          id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
          username TEXT UNIQUE NOT NULL,
          password_hash TEXT NOT NULL,
          email TEXT,
          role TEXT DEFAULT 'user',
          created_at TIMESTAMPTZ DEFAULT NOW()
        );
      `);

      await client.end();
    } else if (storage.type === 'sqlite') {
      const dir = storage.dataDir || this.dataDir;

      // Ensure directory exists
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }

      const Database = (await import('better-sqlite3')).default;
      const db = new Database(path.join(dir, 'topgun.db'));

      db.exec(`
        CREATE TABLE IF NOT EXISTS _system (
          key TEXT PRIMARY KEY,
          value TEXT,
          created_at INTEGER DEFAULT (strftime('%s', 'now')),
          updated_at INTEGER DEFAULT (strftime('%s', 'now'))
        );

        CREATE TABLE IF NOT EXISTS _users (
          id TEXT PRIMARY KEY,
          username TEXT UNIQUE NOT NULL,
          password_hash TEXT NOT NULL,
          email TEXT,
          role TEXT DEFAULT 'user',
          created_at INTEGER DEFAULT (strftime('%s', 'now'))
        );
      `);

      db.close();
    }
  }

  /**
   * Create admin user and return user ID
   */
  private async createAdminUser(
    admin: SetupConfig['admin'],
    storage: SetupConfig['storage']
  ): Promise<string> {
    // Use crypto.scrypt for password hashing (no bcrypt dependency needed)
    const salt = crypto.randomBytes(16).toString('hex');
    const passwordHash = await new Promise<string>((resolve, reject) => {
      crypto.scrypt(admin.password, salt, 64, (err, derivedKey) => {
        if (err) reject(err);
        else resolve(`${salt}:${derivedKey.toString('hex')}`);
      });
    });

    const userId = crypto.randomUUID();

    if (storage.type === 'postgres') {
      const pg = await import('pg');
      const client = new pg.default.Client({ connectionString: storage.connectionString });
      await client.connect();

      await client.query(
        `INSERT INTO _users (id, username, password_hash, email, role)
         VALUES ($1, $2, $3, $4, 'admin')
         ON CONFLICT (username) DO UPDATE SET password_hash = $3`,
        [userId, admin.username, passwordHash, admin.email || null]
      );

      await client.end();
    } else if (storage.type === 'sqlite') {
      const Database = (await import('better-sqlite3')).default;
      const dir = storage.dataDir || this.dataDir;
      const db = new Database(path.join(dir, 'topgun.db'));

      db.prepare(`
        INSERT OR REPLACE INTO _users (id, username, password_hash, email, role)
        VALUES (?, ?, ?, ?, 'admin')
      `).run(userId, admin.username, passwordHash, admin.email || null);

      db.close();
    }

    logger.info({ username: admin.username }, '[Bootstrap] Admin user created');
    return userId;
  }

  /**
   * Save configuration to file
   */
  private async saveConfiguration(config: SetupConfig): Promise<void> {
    const configToSave = {
      version: '1.0',
      deploymentMode: config.deploymentMode,
      storage: {
        type: config.storage.type,
        ...(config.storage.type === 'postgres'
          ? { connectionString: config.storage.connectionString }
          : { dataDir: config.storage.dataDir || this.dataDir }),
      },
      server: {
        port: config.server?.port || 8080,
        metricsPort: config.server?.metricsPort || 9091,
      },
      integrations: {
        mcp: config.integrations?.mcpEnabled
          ? {
              enabled: true,
              port: config.integrations.mcpPort || 3001,
              token: config.integrations.mcpToken || this.generateToken(),
            }
          : { enabled: false },
        vectorSearch: config.integrations?.vectorSearchEnabled
          ? {
              enabled: true,
              model: config.integrations.vectorModel || 'local',
            }
          : { enabled: false },
      },
      createdAt: new Date().toISOString(),
    };

    fs.writeFileSync(this.configPath, JSON.stringify(configToSave, null, 2));
    logger.info({ path: this.configPath }, '[Bootstrap] Configuration saved');
  }

  /**
   * Generate secure token
   */
  private generateToken(): string {
    return crypto.randomBytes(32).toString('hex');
  }

  /**
   * Parse JSON body from request
   */
  private async parseBody(req: IncomingMessage): Promise<any> {
    return new Promise((resolve, reject) => {
      let body = '';
      req.on('data', (chunk) => (body += chunk));
      req.on('end', () => {
        try {
          resolve(JSON.parse(body));
        } catch (err) {
          reject(new Error('Invalid JSON body'));
        }
      });
      req.on('error', reject);
    });
  }

  /**
   * Send JSON response
   */
  private sendJson(res: ServerResponse, status: number, data: unknown): void {
    res.setHeader('Content-Type', 'application/json');
    res.statusCode = status;
    res.end(JSON.stringify(data));
  }
}

/**
 * Create a BootstrapController instance
 */
export function createBootstrapController(config?: BootstrapControllerConfig): BootstrapController {
  return new BootstrapController(config);
}
