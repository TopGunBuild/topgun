/**
 * topgun_mutate - Create, update, or delete data in a TopGun map
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { MutateArgsSchema, toolSchemas } from '../schemas';

export const mutateTool: MCPTool = {
  name: 'topgun_mutate',
  description:
    'Create, update, or delete data in a TopGun map. ' +
    'Use "set" operation to create or update a record. ' +
    'Use "remove" operation to delete a record.',
  inputSchema: toolSchemas.mutate as MCPTool['inputSchema'],
};

export async function handleMutate(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate and parse args with Zod
  const parseResult = MutateArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues.map((e) => `${e.path.join('.')}: ${e.message}`).join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const { map, operation, key, data } = parseResult.data;

  // Check if mutations are enabled
  if (!ctx.config.enableMutations) {
    return {
      content: [
        {
          type: 'text',
          text: 'Error: Mutation operations are disabled on this MCP server.',
        },
      ],
      isError: true,
    };
  }

  // Validate map access
  if (ctx.config.allowedMaps && !ctx.config.allowedMaps.includes(map)) {
    return {
      content: [
        {
          type: 'text',
          text: `Error: Access to map '${map}' is not allowed. Available maps: ${ctx.config.allowedMaps.join(', ')}`,
        },
      ],
      isError: true,
    };
  }

  try {
    const lwwMap = ctx.client.getMap<string, Record<string, unknown>>(map);

    if (operation === 'set') {
      if (!data) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: "data" is required for "set" operation.',
            },
          ],
          isError: true,
        };
      }

      // Add timestamp if not present
      const recordData = {
        ...data,
        _updatedAt: new Date().toISOString(),
      };

      // Check if record exists for logging
      const existingValue = lwwMap.get(key);
      const isCreate = existingValue === undefined;

      lwwMap.set(key, recordData);

      return {
        content: [
          {
            type: 'text',
            text: isCreate
              ? `Successfully created record '${key}' in map '${map}':\n${JSON.stringify(recordData, null, 2)}`
              : `Successfully updated record '${key}' in map '${map}':\n${JSON.stringify(recordData, null, 2)}`,
          },
        ],
      };
    } else if (operation === 'remove') {
      // Check if record exists
      const existingValue = lwwMap.get(key);
      if (existingValue === undefined) {
        return {
          content: [
            {
              type: 'text',
              text: `Warning: Record '${key}' does not exist in map '${map}'. No action taken.`,
            },
          ],
        };
      }

      lwwMap.remove(key);

      return {
        content: [
          {
            type: 'text',
            text: `Successfully removed record '${key}' from map '${map}'.`,
          },
        ],
      };
    }

    return {
      content: [
        {
          type: 'text',
          text: `Error: Invalid operation '${operation}'. Use 'set' or 'remove'.`,
        },
      ],
      isError: true,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error performing ${operation} on map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
