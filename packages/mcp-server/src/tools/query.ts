/**
 * topgun_query - Query data from a TopGun map with filters
 *
 * Uses cursor-based pagination via QueryHandle.
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { QueryArgsSchema, toolSchemas } from '../schemas';

export const queryTool: MCPTool = {
  name: 'topgun_query',
  description:
    'Query data from a TopGun map with filters and sorting. ' +
    'Use this to read data from the database. ' +
    'Supports filtering by field values, sorting, and cursor-based pagination.',
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

  const { map, filter, sort, limit, cursor } = parseResult.data;

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

  try {
    // Build query filter for QueryHandle
    const queryFilter: Record<string, unknown> = {
      where: filter,
      limit: effectiveLimit,
    };

    // Add sort if provided
    if (sort?.field) {
      queryFilter.sort = { [sort.field]: sort.order };
    }

    if (cursor) {
      queryFilter.cursor = cursor;
    }

    // Use QueryHandle for proper server-side query execution
    const handle = ctx.client.query<Record<string, unknown>>(map, queryFilter);

    // Get results via one-shot subscription
    const results = await new Promise<Array<Record<string, unknown> & { _key: string }>>((resolve) => {
      const unsubscribe = handle.subscribe((data) => {
        unsubscribe();
        resolve(data);
      });
    });

    // Get pagination info
    const paginationInfo = handle.getPaginationInfo();

    // Format results
    if (results.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text: `No results found in map '${map}'${filter ? ' matching the filter' : ''}.`,
          },
        ],
      };
    }

    const formatted = results
      .map((entry, idx) => {
        const { _key, ...value } = entry;
        return `${idx + 1}. [${_key}]: ${JSON.stringify(value, null, 2)}`;
      })
      .join('\n\n');

    // Build pagination info for response
    let paginationText = '';
    if (paginationInfo.hasMore && paginationInfo.nextCursor) {
      paginationText = `\n\n---\nMore results available. Use cursor: "${paginationInfo.nextCursor}" to fetch next page.`;
    }

    return {
      content: [
        {
          type: 'text',
          text: `Found ${results.length} result(s) in map '${map}':\n\n${formatted}${paginationText}`,
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
