/**
 * Settings Controller
 *
 * Provides runtime settings management with hot-reload support.
 * Settings are divided into:
 * - Hot-reloadable: Can be changed at runtime without restart
 * - Restart-required: Read-only in UI, require server restart
 */

import { IncomingMessage, ServerResponse } from 'http';
import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import { PARTITION_COUNT } from '@topgunbuild/core';
import { logger } from '../utils/logger';
import { validateJwtSecret } from '../utils/validateConfig';

/**
 * Runtime settings that can be changed without restart
 */
export interface RuntimeSettings {
  logLevel: 'debug' | 'info' | 'warn' | 'error';
  metricsEnabled: boolean;
  rateLimits: {
    connections: number;
    messagesPerSecond: number;
  };
  integrations: {
    mcp: { enabled: boolean };
    vectorSearch: { enabled: boolean };
  };
}

/**
 * Full settings response structure
 */
export interface SettingsResponse {
  general: {
    port: number;
    metricsPort: number;
    logLevel: string;
    version: string;
  };
  storage: {
    type: string;
    connectionString: string | null;
    status: string;
  };
  security: {
    jwtAlgorithm: string;
    sessionTimeout: number;
  };
  integrations: {
    mcp: { enabled: boolean; port: number };
    vectorSearch: { enabled: boolean; model: string | null };
  };
  cluster: {
    mode: string;
    nodeId: string;
    peers: string[];
    partitionCount: number;
  };
  rateLimits: {
    connections: number;
    messagesPerSecond: number;
  };
  _meta: {
    hotReloadable: string[];
    restartRequired: string[];
  };
}

/**
 * Settings that can be hot-reloaded (flattened paths)
 */
const HOT_RELOADABLE = new Set([
  'logLevel',
  'metricsEnabled',
  'rateLimits.connections',
  'rateLimits.messagesPerSecond',
  'integrations.mcp.enabled',
  'integrations.vectorSearch.enabled',
]);

/**
 * Settings that require restart (for documentation in API)
 */
const RESTART_REQUIRED = [
  'port',
  'metricsPort',
  'storage.type',
  'storage.connectionString',
  'cluster.nodeId',
];

/**
 * Configuration file structure (subset of topgun.json)
 */
interface ConfigFile {
  server?: {
    port?: number;
    metricsPort?: number;
  };
  storage?: {
    type?: string;
    connectionString?: string;
    dataDir?: string;
  };
  deploymentMode?: string;
  integrations?: {
    mcp?: {
      enabled?: boolean;
      port?: number;
    };
    vectorSearch?: {
      enabled?: boolean;
      model?: string;
    };
  };
}

export interface SettingsControllerConfig {
  configPath?: string;
  jwtSecret?: string;
  /** Callback when settings change */
  onSettingsChange?: (settings: Partial<RuntimeSettings>) => void;
  /** Function to check storage connection status */
  getStorageStatus?: () => 'connected' | 'disconnected' | 'error';
}

export class SettingsController {
  private configPath: string;
  private jwtSecret: string;
  private runtimeSettings: RuntimeSettings;
  private onSettingsChange?: (settings: Partial<RuntimeSettings>) => void;
  private getStorageStatus?: () => 'connected' | 'disconnected' | 'error';

  constructor(config: SettingsControllerConfig = {}) {
    this.configPath = config.configPath || process.env.TOPGUN_CONFIG_PATH || 'topgun.json';
    this.jwtSecret = validateJwtSecret(config.jwtSecret, process.env.JWT_SECRET);
    this.onSettingsChange = config.onSettingsChange;
    this.getStorageStatus = config.getStorageStatus;
    this.runtimeSettings = this.loadRuntimeSettings();
  }

  /**
   * Get current runtime settings
   */
  getRuntimeSettings(): RuntimeSettings {
    return { ...this.runtimeSettings };
  }

  /**
   * Register callback for settings changes
   */
  setOnSettingsChange(callback: (settings: Partial<RuntimeSettings>) => void): void {
    this.onSettingsChange = callback;
  }

  /**
   * Set storage status checker
   */
  setStorageStatusChecker(checker: () => 'connected' | 'disconnected' | 'error'): void {
    this.getStorageStatus = checker;
  }

  /**
   * Handle HTTP requests for settings endpoints
   */
  async handle(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    const url = req.url || '';
    const method = req.method || 'GET';

    // Check if this is a settings endpoint before doing anything
    const isSettingsEndpoint =
      url === '/api/admin/settings' ||
      url === '/api/admin/settings/validate';

    if (!isSettingsEndpoint) {
      return false;
    }

    // Set CORS headers
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Methods', 'GET, PATCH, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');

    // Handle preflight
    if (method === 'OPTIONS') {
      res.statusCode = 204;
      res.end();
      return true;
    }

    // Verify authentication for all settings endpoints
    const authResult = this.verifyToken(req);
    if (!authResult.valid) {
      this.sendJson(res, 401, { error: 'Unauthorized' });
      return true;
    }

    if (url === '/api/admin/settings' && method === 'GET') {
      return this.handleGetSettings(res);
    }

    if (url === '/api/admin/settings' && method === 'PATCH') {
      return this.handlePatchSettings(req, res);
    }

    if (url === '/api/admin/settings/validate' && method === 'POST') {
      return this.handleValidateSettings(req, res);
    }

    return false;
  }

  /**
   * GET /api/admin/settings
   */
  private handleGetSettings(res: ServerResponse): boolean {
    const config = this.loadConfig();

    const settings: SettingsResponse = {
      general: {
        port: config?.server?.port || parseInt(process.env.TOPGUN_PORT || '8080', 10),
        metricsPort: config?.server?.metricsPort || parseInt(process.env.TOPGUN_METRICS_PORT || '9090', 10),
        logLevel: this.runtimeSettings.logLevel,
        version: process.env.npm_package_version || '0.0.0',
      },
      storage: {
        type: config?.storage?.type || process.env.TOPGUN_STORAGE_TYPE || 'memory',
        connectionString: this.maskConnectionString(
          config?.storage?.connectionString || process.env.DATABASE_URL
        ),
        status: this.getStorageStatus ? this.getStorageStatus() : 'connected',
      },
      security: {
        jwtAlgorithm: 'HS256',
        sessionTimeout: 86400,
      },
      integrations: {
        mcp: {
          enabled: this.runtimeSettings.integrations.mcp.enabled,
          port: config?.integrations?.mcp?.port || parseInt(process.env.TOPGUN_MCP_PORT || '3001', 10),
        },
        vectorSearch: {
          enabled: this.runtimeSettings.integrations.vectorSearch.enabled,
          model: config?.integrations?.vectorSearch?.model || process.env.TOPGUN_VECTOR_MODEL || null,
        },
      },
      cluster: {
        mode: config?.deploymentMode || process.env.TOPGUN_DEPLOYMENT_MODE || 'standalone',
        nodeId: process.env.TOPGUN_NODE_ID || 'node-1',
        peers: [],
        partitionCount: PARTITION_COUNT,
      },
      rateLimits: {
        connections: this.runtimeSettings.rateLimits.connections,
        messagesPerSecond: this.runtimeSettings.rateLimits.messagesPerSecond,
      },
      _meta: {
        hotReloadable: Array.from(HOT_RELOADABLE),
        restartRequired: RESTART_REQUIRED,
      },
    };

    this.sendJson(res, 200, settings);
    return true;
  }

  /**
   * PATCH /api/admin/settings
   */
  private async handlePatchSettings(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    try {
      const body = await this.parseBody(req);

      // Validate first
      const validationErrors = this.validateSettings(body);
      if (validationErrors.length > 0) {
        this.sendJson(res, 400, {
          success: false,
          error: 'Validation failed',
          errors: validationErrors,
        });
        return true;
      }

      // Apply changes
      const { updated, rejected, warnings } = this.applySettings(body);

      if (rejected.length > 0 && updated.length === 0) {
        this.sendJson(res, 400, {
          success: false,
          error: 'Cannot change restart-required settings',
          rejected,
          message: `The following settings require server restart: ${rejected.join(', ')}. Please modify topgun.json and restart.`,
        });
        return true;
      }

      // Persist runtime settings
      if (updated.length > 0) {
        this.saveRuntimeSettings();

        // Notify listeners
        if (this.onSettingsChange) {
          this.onSettingsChange(this.runtimeSettings);
        }

        logger.info({ updated }, '[Settings] Settings updated');
      }

      this.sendJson(res, 200, {
        success: true,
        updated,
        rejected,
        warnings,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unknown error';
      this.sendJson(res, 400, { success: false, error: message });
    }

    return true;
  }

  /**
   * POST /api/admin/settings/validate
   */
  private async handleValidateSettings(req: IncomingMessage, res: ServerResponse): Promise<boolean> {
    try {
      const body = await this.parseBody(req);
      const errors = this.validateSettings(body);

      this.sendJson(res, 200, {
        valid: errors.length === 0,
        errors,
      });
    } catch {
      this.sendJson(res, 400, {
        valid: false,
        errors: [{ path: '', message: 'Invalid request body', value: null }],
      });
    }

    return true;
  }

  /**
   * Apply settings changes
   */
  private applySettings(changes: Record<string, unknown>): {
    updated: string[];
    rejected: string[];
    warnings: string[];
  } {
    const updated: string[] = [];
    const rejected: string[] = [];
    const warnings: string[] = [];

    const flatChanges = this.flattenObject(changes);

    for (const [path, value] of Object.entries(flatChanges)) {
      if (!HOT_RELOADABLE.has(path)) {
        rejected.push(path);
        continue;
      }

      // Apply the change
      this.setNestedValue(this.runtimeSettings, path, value);
      updated.push(path);
    }

    return { updated, rejected, warnings };
  }

  /**
   * Validate settings values
   */
  private validateSettings(changes: Record<string, unknown>): Array<{
    path: string;
    message: string;
    value: unknown;
  }> {
    const errors: Array<{ path: string; message: string; value: unknown }> = [];
    const flatChanges = this.flattenObject(changes);

    for (const [path, value] of Object.entries(flatChanges)) {
      // Log level validation
      if (path === 'logLevel') {
        const validLevels = ['debug', 'info', 'warn', 'error'];
        if (!validLevels.includes(value as string)) {
          errors.push({
            path,
            message: `Must be one of: ${validLevels.join(', ')}`,
            value,
          });
        }
      }

      // Boolean validation for enabled flags
      if (path === 'metricsEnabled' || path.endsWith('.enabled')) {
        if (typeof value !== 'boolean') {
          errors.push({
            path,
            message: 'Must be a boolean',
            value,
          });
        }
      }

      // Rate limits validation - must be >= 1 to prevent blocking all connections
      if (path.startsWith('rateLimits.')) {
        if (typeof value !== 'number' || value < 1) {
          errors.push({
            path,
            message: 'Must be a positive number (minimum 1)',
            value,
          });
        }
      }
    }

    return errors;
  }

  /**
   * Load runtime settings from file or defaults
   */
  private loadRuntimeSettings(): RuntimeSettings {
    const defaultSettings: RuntimeSettings = {
      logLevel: (process.env.LOG_LEVEL as RuntimeSettings['logLevel']) || 'info',
      metricsEnabled: true,
      rateLimits: {
        connections: 1000,
        messagesPerSecond: 10000,
      },
      integrations: {
        mcp: { enabled: process.env.TOPGUN_MCP_ENABLED === 'true' },
        vectorSearch: { enabled: process.env.TOPGUN_VECTOR_ENABLED === 'true' },
      },
    };

    const runtimePath = this.getRuntimePath();
    if (fs.existsSync(runtimePath)) {
      try {
        const saved = JSON.parse(fs.readFileSync(runtimePath, 'utf-8'));
        return this.mergeSettings(defaultSettings, saved);
      } catch {
        logger.warn({ path: runtimePath }, '[Settings] Failed to load runtime settings, using defaults');
        return defaultSettings;
      }
    }

    return defaultSettings;
  }

  /**
   * Persist runtime settings to file
   */
  private saveRuntimeSettings(): void {
    const runtimePath = this.getRuntimePath();
    try {
      fs.writeFileSync(runtimePath, JSON.stringify(this.runtimeSettings, null, 2));
      logger.debug({ path: runtimePath }, '[Settings] Runtime settings saved');
    } catch (error) {
      logger.error({ error, path: runtimePath }, '[Settings] Failed to save runtime settings');
    }
  }

  /**
   * Get path for runtime settings file
   */
  private getRuntimePath(): string {
    return this.configPath.replace(/\.json$/, '.runtime.json');
  }

  /**
   * Load main config file
   */
  private loadConfig(): ConfigFile | null {
    if (fs.existsSync(this.configPath)) {
      try {
        return JSON.parse(fs.readFileSync(this.configPath, 'utf-8')) as ConfigFile;
      } catch {
        return null;
      }
    }
    return null;
  }

  /**
   * Mask sensitive parts of connection string
   */
  private maskConnectionString(str?: string | null): string | null {
    if (!str) return null;
    // Mask password in URL: postgres://user:password@host -> postgres://user:***@host
    return str.replace(/:([^:@]+)@/, ':***@');
  }

  /**
   * Verify JWT token from Authorization header
   */
  private verifyToken(req: IncomingMessage): { valid: boolean; principal?: Record<string, unknown> } {
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
   * Merge saved settings with defaults (handles schema changes)
   */
  private mergeSettings(defaults: RuntimeSettings, saved: Partial<RuntimeSettings>): RuntimeSettings {
    return {
      logLevel: saved.logLevel || defaults.logLevel,
      metricsEnabled: saved.metricsEnabled ?? defaults.metricsEnabled,
      rateLimits: {
        connections: saved.rateLimits?.connections ?? defaults.rateLimits.connections,
        messagesPerSecond: saved.rateLimits?.messagesPerSecond ?? defaults.rateLimits.messagesPerSecond,
      },
      integrations: {
        mcp: {
          enabled: saved.integrations?.mcp?.enabled ?? defaults.integrations.mcp.enabled,
        },
        vectorSearch: {
          enabled: saved.integrations?.vectorSearch?.enabled ?? defaults.integrations.vectorSearch.enabled,
        },
      },
    };
  }

  /**
   * Flatten nested object to dot-notation paths
   */
  private flattenObject(obj: Record<string, unknown>, prefix = ''): Record<string, unknown> {
    const result: Record<string, unknown> = {};

    for (const key of Object.keys(obj)) {
      const path = prefix ? `${prefix}.${key}` : key;
      const value = obj[key];

      if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
        Object.assign(result, this.flattenObject(value as Record<string, unknown>, path));
      } else {
        result[path] = value;
      }
    }

    return result;
  }

  /**
   * Set nested value by dot-notation path
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private setNestedValue(obj: any, path: string, value: unknown): void {
    const keys = path.split('.');
    let current = obj;

    for (let i = 0; i < keys.length - 1; i++) {
      const key = keys[i];
      if (!(key in current) || typeof current[key] !== 'object') {
        current[key] = {};
      }
      current = current[key];
    }

    current[keys[keys.length - 1]] = value;
  }

  /**
   * Parse JSON body from request
   */
  private async parseBody(req: IncomingMessage): Promise<Record<string, unknown>> {
    return new Promise((resolve, reject) => {
      let body = '';
      req.on('data', (chunk) => (body += chunk));
      req.on('end', () => {
        try {
          resolve(JSON.parse(body));
        } catch {
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
 * Create a SettingsController instance
 */
export function createSettingsController(config?: SettingsControllerConfig): SettingsController {
  return new SettingsController(config);
}
