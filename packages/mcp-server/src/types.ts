/**
 * TopGun MCP Server Types
 */

import type { TopGunClient } from '@topgunbuild/client';

/**
 * Configuration for the MCP server
 */
export interface MCPServerConfig {
  /**
   * Name of the MCP server (shown to AI clients)
   * @default 'topgun-mcp-server'
   */
  name?: string;

  /**
   * Version of the MCP server
   * @default '1.0.0'
   */
  version?: string;

  /**
   * TopGun client instance to use
   * If not provided, a new client will be created with topgunUrl
   */
  client?: TopGunClient;

  /**
   * TopGun server URL (WebSocket)
   * Used when client is not provided
   * @default 'ws://localhost:8080'
   */
  topgunUrl?: string;

  /**
   * Authentication token for TopGun server
   */
  authToken?: string;

  /**
   * Restrict available maps (if not specified, all maps are available)
   */
  allowedMaps?: string[];

  /**
   * Enable mutation tools (set, remove)
   * @default true
   */
  enableMutations?: boolean;

  /**
   * Enable subscription tools
   * @default true
   */
  enableSubscriptions?: boolean;

  /**
   * Default limit for query results
   * @default 10
   */
  defaultLimit?: number;

  /**
   * Maximum limit for query results
   * @default 100
   */
  maxLimit?: number;

  /**
   * Subscription timeout in seconds
   * @default 60
   */
  subscriptionTimeoutSeconds?: number;

  /**
   * Enable debug logging
   * @default false
   */
  debug?: boolean;
}

/**
 * Resolved configuration with all defaults applied
 */
export interface ResolvedMCPServerConfig {
  name: string;
  version: string;
  topgunUrl: string;
  authToken?: string;
  allowedMaps?: string[];
  enableMutations: boolean;
  enableSubscriptions: boolean;
  defaultLimit: number;
  maxLimit: number;
  subscriptionTimeoutSeconds: number;
  debug: boolean;
}

/**
 * MCP Tool definition
 */
export interface MCPTool {
  name: string;
  description: string;
  inputSchema: {
    type: 'object';
    properties: Record<string, unknown>;
    required?: string[];
  };
}

/**
 * MCP Tool call result
 */
export interface MCPToolResult {
  content: Array<{
    type: 'text' | 'image' | 'resource';
    text?: string;
    data?: string;
    mimeType?: string;
  }>;
  isError?: boolean;
}

/**
 * Query tool arguments
 */
export interface QueryToolArgs {
  map: string;
  filter?: Record<string, unknown>;
  sort?: {
    field: string;
    order: 'asc' | 'desc';
  };
  limit?: number;
  cursor?: string;
}

/**
 * Mutate tool arguments
 */
export interface MutateToolArgs {
  map: string;
  operation: 'set' | 'remove';
  key: string;
  data?: Record<string, unknown>;
}

/**
 * Search tool arguments
 */
export interface SearchToolArgs {
  map: string;
  query: string;
  methods?: Array<'exact' | 'fulltext' | 'range'>;
  limit?: number;
  minScore?: number;
}

/**
 * Subscribe tool arguments
 */
export interface SubscribeToolArgs {
  map: string;
  filter?: Record<string, unknown>;
  timeout?: number;
}

/**
 * Schema tool arguments
 */
export interface SchemaToolArgs {
  map: string;
}

/**
 * Stats tool arguments
 */
export interface StatsToolArgs {
  map?: string;
}

/**
 * Explain tool arguments
 */
export interface ExplainToolArgs {
  map: string;
  filter?: Record<string, unknown>;
}

/**
 * List maps tool arguments (no args required)
 */
export interface ListMapsToolArgs {
  // No arguments needed
}

/**
 * Tool execution context
 */
export interface ToolContext {
  client: TopGunClient;
  config: ResolvedMCPServerConfig;
}
