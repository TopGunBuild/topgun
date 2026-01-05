/**
 * topgun_list_maps - List all available maps
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { ListMapsArgsSchema, toolSchemas } from '../schemas';

export const listMapsTool: MCPTool = {
  name: 'topgun_list_maps',
  description:
    'List all available TopGun maps that can be queried. ' +
    'Returns the names of maps you have access to. ' +
    'Use this first to discover what data is available.',
  inputSchema: toolSchemas.listMaps as MCPTool['inputSchema'],
};

export async function handleListMaps(
  rawArgs: unknown,
  ctx: ToolContext
): Promise<MCPToolResult> {
  // Validate arguments with Zod (no required fields, but validates structure)
  const parseResult = ListMapsArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues.map((e) => `${e.path.join('.')}: ${e.message}`).join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }
  try {
    // If allowedMaps is configured, return those
    if (ctx.config.allowedMaps && ctx.config.allowedMaps.length > 0) {
      const mapList = ctx.config.allowedMaps.map((name) => `  - ${name}`).join('\n');

      return {
        content: [
          {
            type: 'text',
            text:
              `Available maps (${ctx.config.allowedMaps.length}):\n${mapList}\n\n` +
              `Use topgun_schema to get field information for a specific map.\n` +
              `Use topgun_query to read data from a map.`,
          },
        ],
      };
    }

    // Otherwise, indicate that all maps are accessible but we don't have a directory
    return {
      content: [
        {
          type: 'text',
          text:
            `This MCP server allows access to all maps (no restrictions configured).\n\n` +
            `To query a map, use topgun_query with the map name.\n` +
            `To get schema information, use topgun_schema.\n` +
            `To search, use topgun_search.\n\n` +
            `Common map patterns:\n` +
            `  - 'users' - User accounts\n` +
            `  - 'tasks' - Task items\n` +
            `  - 'posts' - Blog posts or messages\n` +
            `  - 'products' - E-commerce products\n\n` +
            `Tip: Ask the user what maps are available in their application.`,
          },
        ],
      };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error listing maps: ${message}`,
        },
      ],
      isError: true,
    };
  }
}
