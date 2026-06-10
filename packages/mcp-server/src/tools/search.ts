/**
 * topgun_search — Tri-hybrid search (exact + full-text BM25 + semantic) via RRF fusion.
 * Routes through the hybridSearch client method so the tool actually performs what it advertises.
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { SearchArgsSchema, toolSchemas, type SearchArgs } from '../schemas';

export const searchTool: MCPTool = {
  name: 'topgun_search',
  description:
    'Search a TopGun map combining exact, full-text (BM25), and semantic methods, ' +
    'fused with Reciprocal Rank Fusion. ' +
    'Defaults to full-text only. ' +
    'Pass methods: ["exact","fullText"] or ["fullText","semantic"] to enable additional legs. ' +
    '"semantic" requires server-side auto-embedding (the tool sends a text query, not a vector). ' +
    'Returns results ranked by a fused relevance score with per-method score breakdown.',
  inputSchema: toolSchemas.search as MCPTool['inputSchema'],
};

export async function handleSearch(rawArgs: unknown, ctx: ToolContext): Promise<MCPToolResult> {
  // Validate arguments with Zod
  const parseResult = SearchArgsSchema.safeParse(rawArgs);
  if (!parseResult.success) {
    const errors = parseResult.error.issues
      .map((e) => `${e.path.join('.')}: ${e.message}`)
      .join(', ');
    return {
      content: [{ type: 'text', text: `Invalid arguments: ${errors}` }],
      isError: true,
    };
  }

  const args: SearchArgs = parseResult.data;
  const { map, query, limit, minScore, methods } = args;

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

  // Map the existing `limit` arg to hybridSearch's `k` so agents keep using the same param name.
  const effectiveLimit = Math.min(limit ?? ctx.config.defaultLimit, ctx.config.maxLimit);
  const effectiveMethods = methods ?? ['fullText'];
  const effectiveMinScore = minScore ?? 0;

  try {
    const results = await ctx.client.hybridSearch(map, query, {
      methods: effectiveMethods,
      k: effectiveLimit,
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

    // Obtain a single map handle before the loop — same accessor topgun_mutate uses —
    // so each hit's body is read from the local CRDT replica without a network round-trip.
    const lwwMap = ctx.client.getMap<string, Record<string, unknown>>(map);

    const formatted = results
      .map((result, idx) => {
        const body = lwwMap.get(result.key);
        const dataLine =
          body !== undefined
            ? `Data: ${JSON.stringify(body, null, 2).split('\n').join('\n   ')}`
            : `Data: (record body not available locally)`;

        // Render per-method score breakdown so the calling agent can see which leg contributed.
        const methodScoreEntries = Object.entries(result.methodScores)
          .map(([method, score]) => `${method}: ${(score as number).toFixed(3)}`)
          .join(', ');
        const methodLine = methodScoreEntries
          ? `Method scores: ${methodScoreEntries}`
          : 'Method scores: (none)';

        return (
          `${idx + 1}. [Score: ${result.score.toFixed(3)}] [${result.key}]\n` +
          `   ${methodLine}\n` +
          `   ${dataLine}`
        );
      })
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

    // When the server cannot embed the query for semantic search, surface an actionable message
    // so the calling agent knows to retry without the semantic leg.
    if (
      message.toLowerCase().includes('embed') ||
      (effectiveMethods.includes('semantic') &&
        (message.includes('semantic') || message.includes('vector')))
    ) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Semantic search requires server-side embedding, which is not available for map '${map}'. ` +
              `Retry with methods: ["fullText"] or methods: ["exact", "fullText"] to avoid the semantic leg.`,
          },
        ],
        isError: true,
      };
    }

    // Handle case where full-text search is not enabled for the map
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
