/**
 * topgun_query - Query data from a TopGun map with filters
 *
 * Resolves on the first settled server snapshot via client.queryOnce().
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
    'Supports filtering by field values, sorting, and pagination via limit.',
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

  const { map, filter, sort, limit, fields } = parseResult.data;

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
    // Build query filter for QueryHandle. Fetch ONE extra row beyond the
    // effective limit so we can detect (and signal) that results were truncated.
    // queryOnce returns a plain array with no hasMore metadata, so without this
    // probe an agent that asks for `limit` rows and gets exactly `limit` back
    // cannot tell a complete result from a capped one — and would silently report
    // a truncated view as the whole answer.
    const queryFilter: QueryFilter = {
      where: filter,
      limit: effectiveLimit + 1,
    };

    // Add sort if provided
    if (sort?.field) {
      queryFilter.sort = { [sort.field]: sort.order };
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

    // We requested effectiveLimit + 1 rows; if the server had more, drop the
    // probe row and remember to tell the caller the result was capped.
    const truncated = results.length > effectiveLimit;
    if (truncated) {
      results = results.slice(0, effectiveLimit);
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

    // Honest truncation signal: there is intentionally no cursor to page through
    // (continuation cursors are an anti-pattern for LLM callers), so point the
    // agent at narrowing the query instead — and at raising `limit` only when it
    // is still below the server's maxLimit cap.
    const truncationNote = truncated
      ? `\n\n---\nMore rows match than were returned; showing the first ${effectiveLimit}. ` +
        `Narrow with \`filter\`/\`sort\`` +
        (effectiveLimit < ctx.config.maxLimit
          ? ` or raise \`limit\` (up to ${ctx.config.maxLimit})`
          : '') +
        ` to see the rest. There is no cursor to page through.`
      : '';

    return {
      content: [
        {
          type: 'text',
          text: `Found ${results.length} result(s) in map '${map}':\n\n${formatted}${truncationNote}`,
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
