/**
 * @topgunbuild/mcp-server
 *
 * MCP Server for TopGun - enables AI assistants to interact with TopGun databases.
 *
 * @packageDocumentation
 */

// Main server class
export { TopGunMCPServer } from './TopGunMCPServer';

// Types
export type {
  MCPServerConfig,
  ResolvedMCPServerConfig,
  MCPTool,
  MCPToolResult,
  QueryToolArgs,
  MutateToolArgs,
  SearchToolArgs,
  SubscribeToolArgs,
  SchemaToolArgs,
  StatsToolArgs,
  ExplainToolArgs,
  ListMapsToolArgs,
  ToolContext,
} from './types';

// Transport
export { HTTPTransport, createHTTPServer } from './transport';
export type { HTTPServerConfig } from './transport';

// Tools (for advanced usage)
export {
  allTools,
  toolHandlers,
  queryTool,
  mutateTool,
  searchTool,
  subscribeTool,
  schemaTool,
  statsTool,
  explainTool,
  listMapsTool,
} from './tools';
