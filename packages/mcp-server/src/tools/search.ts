/**
 * topgun_search - Perform hybrid search (exact + full-text) across a map
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { SearchArgsSchema, toolSchemas, type SearchArgs } from '../schemas';

export const searchTool: MCPTool = {
  name: 'topgun_search',
  description:
    'Perform hybrid search across a TopGun map using BM25 full-text search. ' +
    'Returns results ranked by relevance score. ' +
    'Use this when searching for text content or when the exact field values are unknown.',
  inputSchema: toolSchemas.search as MCPTool['inputSchema'],
};

export async function handleSearch(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate arguments with Zod
  const parseResult = SearchArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues.map((e) => `${e.path.join('.')}: ${e.message}`).join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const args: SearchArgs = parseResult.data;
  const { map, query, limit, minScore } = args;

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

  const effectiveLimit = Math.min(limit ?? ctx.config.defaultLimit, ctx.config.maxLimit);
  const effectiveMinScore = minScore ?? 0;

  try {
    const results = await ctx.client.search<Record<string, unknown>>(map, query, {
      limit: effectiveLimit,
      minScore: effectiveMinScore,
    });

    if (results.length === 0) {
      return {
        content: [
          {
            type: 'text',
            text: `No results found in map '${map}' for query "${query}".`,
          },
        ],
      };
    }

    const formatted = results
      .map(
        (result, idx) =>
          `${idx + 1}. [Score: ${result.score.toFixed(3)}] [${result.key}]\n` +
          `   Matched: ${result.matchedTerms.join(', ')}\n` +
          `   Data: ${JSON.stringify(result.value, null, 2).split('\n').join('\n   ')}`
      )
      .join('\n\n');

    return {
      content: [
        {
          type: 'text',
          text: `Found ${results.length} result(s) in map '${map}' for query "${query}":\n\n${formatted}`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);

    // Handle case where FTS is not enabled for the map
    if (message.includes('not enabled') || message.includes('FTS')) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Full-text search is not enabled for map '${map}'. ` +
              `Use topgun_query instead for exact matching, or enable FTS on the server.`,
          },
        ],
        isError: true,
      };
    }

    return {
      content: [
        {
          type: 'text',
          text: `Error searching map '${map}': ${message}`,
        },
      ],
      isError: true,
    };
  }
}
