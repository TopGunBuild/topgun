/**
 * topgun_query - Query data from a TopGun map with filters
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { QueryArgsSchema, toolSchemas, type QueryArgs } from '../schemas';

export const queryTool: MCPTool = {
  name: 'topgun_query',
  description:
    'Query data from a TopGun map with filters and sorting. ' +
    'Use this to read data from the database. ' +
    'Supports filtering by field values and sorting by any field.',
  inputSchema: toolSchemas.query as MCPTool['inputSchema'],
};

export async function handleQuery(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate and parse args with Zod
  const parseResult = QueryArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues.map((e) => `${e.path.join('.')}: ${e.message}`).join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const { map, filter, sort, limit, offset } = parseResult.data;

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
