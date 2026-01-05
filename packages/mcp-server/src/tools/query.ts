/**
 * topgun_query - Query data from a TopGun map with filters
 */

import type { MCPTool, MCPToolResult, QueryToolArgs, ToolContext } from '../types';

export const queryTool: MCPTool = {
  name: 'topgun_query',
  description:
    'Query data from a TopGun map with filters and sorting. ' +
    'Use this to read data from the database. ' +
    'Supports filtering by field values and sorting by any field.',
  inputSchema: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: "Name of the map to query (e.g., 'tasks', 'users', 'products')",
      },
      filter: {
        type: 'object',
        description:
          'Filter criteria as key-value pairs. ' +
          'Example: { "status": "active", "priority": "high" }',
        additionalProperties: true,
      },
      sort: {
        type: 'object',
        description: 'Sort configuration',
        properties: {
          field: {
            type: 'string',
            description: 'Field name to sort by',
          },
          order: {
            type: 'string',
            enum: ['asc', 'desc'],
            description: 'Sort order: ascending or descending',
          },
        },
      },
      limit: {
        type: 'number',
        description: 'Maximum number of results to return',
        default: 10,
      },
      offset: {
        type: 'number',
        description: 'Number of results to skip (for pagination)',
        default: 0,
      },
    },
    required: ['map'],
  },
};

export async function handleQuery(args: QueryToolArgs, ctx: ToolContext): Promise<MCPToolResult> {
  const { map, filter, sort, limit, offset } = args;

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

  // Apply limits
  const effectiveLimit = Math.min(limit ?? ctx.config.defaultLimit, ctx.config.maxLimit);
  const effectiveOffset = offset ?? 0;

  try {
    // Get the map and query
    const lwwMap = ctx.client.getMap<string, Record<string, unknown>>(map);

    // Get all entries and apply filter/sort/pagination client-side
    // Note: In production, this should use server-side indexing
    const allEntries: Array<{ key: string; value: Record<string, unknown> }> = [];

    // For now, we iterate over the map's entries
    // This is a simplified implementation - real implementation would use QueryHandle
    for (const [key, value] of lwwMap.entries()) {
      if (value !== null && typeof value === 'object') {
        // Apply filter
        let matches = true;
        if (filter) {
          for (const [filterKey, filterValue] of Object.entries(filter)) {
            if ((value as Record<string, unknown>)[filterKey] !== filterValue) {
              matches = false;
              break;
            }
          }
        }
        if (matches) {
          allEntries.push({ key: String(key), value: value as Record<string, unknown> });
        }
      }
    }

    // Apply sort
    if (sort?.field) {
      allEntries.sort((a, b) => {
        const aVal = a.value[sort.field];
        const bVal = b.value[sort.field];

        if (aVal === bVal) return 0;
        if (aVal === undefined || aVal === null) return 1;
        if (bVal === undefined || bVal === null) return -1;

        const comparison = aVal < bVal ? -1 : 1;
        return sort.order === 'desc' ? -comparison : comparison;
      });
    }

    // Apply pagination
    const paginatedEntries = allEntries.slice(effectiveOffset, effectiveOffset + effectiveLimit);

    // Format results
    if (paginatedEntries.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text: `No results found in map '${map}'${filter ? ' matching the filter' : ''}.`,
          },
        ],
      };
    }

    const formatted = paginatedEntries
      .map((entry, idx) => `${idx + 1 + effectiveOffset}. [${entry.key}]: ${JSON.stringify(entry.value, null, 2)}`)
      .join('\n\n');

    const totalInfo =
      allEntries.length > effectiveLimit
        ? `\n\n(Showing ${effectiveOffset + 1}-${effectiveOffset + paginatedEntries.length} of ${allEntries.length} total)`
        : '';

    return {
      content: [
        {
          type: 'text',
          text: `Found ${paginatedEntries.length} result(s) in map '${map}':\n\n${formatted}${totalInfo}`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      content: [
        {
          type: 'text',
          text: `Error querying map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
