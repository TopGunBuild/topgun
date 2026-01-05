/**
 * MCP Tools Index
 * Exports all tool definitions and handlers
 */

export { queryTool, handleQuery } from './query';
export { mutateTool, handleMutate } from './mutate';
export { searchTool, handleSearch } from './search';
export { subscribeTool, handleSubscribe } from './subscribe';
export { schemaTool, handleSchema } from './schema';
export { statsTool, handleStats } from './stats';
export { explainTool, handleExplain } from './explain';
export { listMapsTool, handleListMaps } from './listMaps';

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { queryTool, handleQuery } from './query';
import { mutateTool, handleMutate } from './mutate';
import { searchTool, handleSearch } from './search';
import { subscribeTool, handleSubscribe } from './subscribe';
import { schemaTool, handleSchema } from './schema';
import { statsTool, handleStats } from './stats';
import { explainTool, handleExplain } from './explain';
import { listMapsTool, handleListMaps } from './listMaps';

/**
 * All available tools
 */
export const allTools: MCPTool[] = [
  listMapsTool,
  queryTool,
  mutateTool,
  searchTool,
  subscribeTool,
  schemaTool,
  statsTool,
  explainTool,
];

/**
 * Tool handlers map
 */
export const toolHandlers: Record<
  string,
  (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>
> = {
  topgun_list_maps: handleListMaps as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_query: handleQuery as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_mutate: handleMutate as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_search: handleSearch as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_subscribe: handleSubscribe as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_schema: handleSchema as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_stats: handleStats as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
  topgun_explain: handleExplain as (args: unknown, ctx: ToolContext) => Promise<MCPToolResult>,
};
