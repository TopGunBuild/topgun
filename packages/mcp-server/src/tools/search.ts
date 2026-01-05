/**
 * topgun_search - Perform hybrid search (exact + full-text) across a map
 */

import type { MCPTool, MCPToolResult, SearchToolArgs, ToolContext } from '../types';

export const searchTool: MCPTool = {
  name: 'topgun_search',
  description:
    'Perform hybrid search across a TopGun map using BM25 full-text search. ' +
    'Returns results ranked by relevance score. ' +
    'Use this when searching for text content or when the exact field values are unknown.',
  inputSchema: {
    type: 'object',
    properties: {
      map: {
        type: 'string',
        description: "Name of the map to search (e.g., 'articles', 'documents', 'tasks')",
      },
      query: {
        type: 'string',
        description: 'Search query (keywords or phrases to find)',
      },
      methods: {
        type: 'array',
        items: {
          type: 'string',
          enum: ['exact', 'fulltext', 'range'],
        },
        description: 'Search methods to use. Default: ["exact", "fulltext"]',
        default: ['exact', 'fulltext'],
      },
      limit: {
        type: 'number',
        description: 'Maximum number of results to return',
        default: 10,
      },
      minScore: {
        type: 'number',
        description: 'Minimum relevance score (0-1) for results',
        default: 0,
      },
    },
    required: ['map', 'query'],
  },
};

export async function handleSearch(args: SearchToolArgs, ctx: ToolContext): Promise<MCPToolResult> {
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
    // Use the client's search API (Phase 11)
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
