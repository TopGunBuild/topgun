/**
 * topgun_query - Query data from a TopGun map with filters
 *
 * Uses cursor-based pagination via QueryHandle.
 */

import type { QueryFilter } from '@topgunbuild/client';
import { QueryOnceUnsettledError } from '@topgunbuild/client';
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
    const errors = parseResult.error.issues
      .map((e) => `${e.path.join('.')}: ${e.message}`)
      .join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const { map, filter, sort, limit, cursor, fields } = parseResult.data;

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
    const queryFilter: QueryFilter = {
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

    if (fields && fields.length > 0) {
      queryFilter.fields = fields;
    }

    // queryOnce returns settled, authoritative server data on a normal resolve.
    // Using the default (no allowLocal) is the strict server-truth contract: it
    // never silently returns stale local data — on offline/timeout it rejects
    // with QueryOnceUnsettledError, which we surface explicitly below so an
    // unreachable server is never confused with a genuinely empty result.
    let results: Array<Record<string, unknown> & { _key: string }>;
    try {
      results = await ctx.client.queryOnce<Record<string, unknown>>(map, queryFilter);
    } catch (error) {
      if (error instanceof QueryOnceUnsettledError) {
        const why =
          error.reason === 'offline'
            ? 'the server could not be reached (client is offline)'
            : 'the query did not settle in time (timed out waiting for the server)';
        return {
          content: [
            {
              type: 'text',
              text:
                `Could not query map '${map}': results not settled — ${why}. ` +
                `No authoritative server data was returned, so this is NOT an empty result. ` +
                `Check connectivity and retry.`,
            },
          ],
          isError: true,
        };
      }
      throw error;
    }

    // A settled-but-empty result is a legitimate "no matching records" answer,
    // distinct from the offline/not-settled branch handled above.
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

    return {
      content: [
        {
          type: 'text',
          text: `Found ${results.length} result(s) in map '${map}':\n\n${formatted}`,
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
