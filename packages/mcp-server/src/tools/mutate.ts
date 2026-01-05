/**
 * topgun_mutate - Create, update, or delete data in a TopGun map
 */

import type { MCPTool, MCPToolResult, MutateToolArgs, ToolContext } from '../types';

export const mutateTool: MCPTool = {
  name: 'topgun_mutate',
  description:
    'Create, update, or delete data in a TopGun map. ' +
    'Use "set" operation to create or update a record. ' +
    'Use "remove" operation to delete a record.',
  inputSchema: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: "Name of the map to modify (e.g., 'tasks', 'users')",
      },
      operation: {
        type: 'string',
        enum: ['set', 'remove'],
        description: '"set" creates or updates a record, "remove" deletes it',
      },
      key: {
        type: 'string',
        description: 'Unique key for the record',
      },
      data: {
        type: 'object',
        description: 'Data to write (required for "set" operation)',
        additionalProperties: true,
      },
    },
    required: ['map', 'operation', 'key'],
  },
};

export async function handleMutate(args: MutateToolArgs, ctx: ToolContext): Promise<MCPToolResult> {
  const { map, operation, key, data } = args;

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
