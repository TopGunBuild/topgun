/**
 * topgun_search — Hybrid search (exact + full-text BM25) via RRF fusion.
 * Routes through the hybridSearch client method so the tool actually performs what it advertises.
 *
 * Result bodies are read server-authoritatively (the same settled server read
 * topgun_query uses), NOT from the MCP process's local CRDT replica. That replica
 * is only ever populated by topgun_mutate writes made in this process, so for any
 * record the agent did not write itself it is cold — a body read from it renders
 * "(not available)" even when the server holds the real record.
 *
 * The "semantic" (vector) leg is not yet functional on the server, so it is not
 * advertised and a semantic-only request is rejected honestly rather than silently
 * routed to a dead leg.
 */

import type { MCPTool, MCPToolResult, ToolContext } from '../types';
import { SearchArgsSchema, toolSchemas, type SearchArgs } from '../schemas';
import { fetchServerRecordsByKeys, ServerReadUnsettledError } from './serverRead';

export const searchTool: MCPTool = {
  name: 'topgun_search',
  description:
    'Search a TopGun map combining exact and full-text (BM25) methods, ' +
    'fused with Reciprocal Rank Fusion. ' +
    'Defaults to full-text only. ' +
    'Pass methods: ["exact","fullText"] to combine both legs. ' +
    'Result bodies are read authoritatively from the server. ' +
    'Returns results ranked by a fused relevance score with per-method score breakdown. ' +
    '(Semantic / vector search is not yet available on the server.)',
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
  const requestedMethods = methods ?? ['fullText'];
  const effectiveMinScore = minScore ?? 0;

  // The semantic (vector) leg is dark on the server: routing a request to it would
  // silently contribute nothing or stall on a dead embedding path. Drop it and tell
  // the agent honestly rather than pretending it ran. When the remaining methods are
  // empty (a semantic-only request) there is nothing to run, so reject outright.
  const semanticRequested = requestedMethods.includes('semantic');
  const effectiveMethods = requestedMethods.filter((m) => m !== 'semantic');
  if (effectiveMethods.length === 0) {
    return {
      content: [
        {
          type: 'text',
          text:
            `Semantic (vector) search is not yet available on this server. ` +
            `Retry with methods: ["fullText"] or methods: ["exact", "fullText"].`,
        },
      ],
      isError: true,
    };
  }

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

    // hybridSearch ranks keys but does not return record bodies. Hydrate them from a
    // settled, server-authoritative read keyed by exactly the hit keys (a single
    // `_key IN (...)` query), instead of the cold local replica, so each hit renders
    // the real server body. Keying by the hits keeps this O(hits) and — unlike a
    // first-page scan — never marks a real record "not available" just because its
    // key sorted beyond a page boundary.
    //
    // A hydration failure must NOT be reclassified as a search/FTS error (ranking
    // already succeeded): handle it here and degrade the bodies, rather than letting
    // it fall through to the outer catch's FTS/index matcher.
    let serverBodies = new Map<string, Record<string, unknown>>();
    let hydrationFailure: 'unsettled' | 'failed' | null = null;
    try {
      serverBodies = await fetchServerRecordsByKeys(
        ctx,
        map,
        results.map((r) => r.key),
      );
    } catch (hydrationError) {
      hydrationFailure =
        hydrationError instanceof ServerReadUnsettledError ? 'unsettled' : 'failed';
    }

    const formatted = results
      .map((result, idx) => {
        const body = serverBodies.get(result.key);
        let dataLine: string;
        if (body !== undefined) {
          dataLine = `Data: ${JSON.stringify(body, null, 2).split('\n').join('\n   ')}`;
        } else if (hydrationFailure === 'unsettled') {
          dataLine = `Data: (record body not fetched — server read did not settle; retry)`;
        } else if (hydrationFailure === 'failed') {
          dataLine = `Data: (record body not fetched — server read failed; retry)`;
        } else {
          dataLine = `Data: (record body not available on the server)`;
        }

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

    // When the agent asked for the semantic leg, tell it the leg was skipped so the
    // ranking is never silently narrower than requested.
    const semanticNote = semanticRequested
      ? `\n\n(Note: the "semantic" method was skipped — vector search is not yet available on this server. ` +
        `Results above use only ${effectiveMethods.map((m) => `"${m}"`).join(', ')}.)`
      : '';

    return {
      content: [
        {
          type: 'text',
          text: `Found ${results.length} result(s) in map '${map}' for query "${query}":\n\n${formatted}${semanticNote}`,
        },
      ],
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);

    // Full-text search index is absent for this map. The server surfaces this as
    // several low-level strings ("FTS not enabled", "index registry not found for
    // map", "no index"); map them all to one actionable message instead of leaking
    // the raw internal error to the agent.
    if (
      message.includes('not enabled') ||
      message.includes('FTS') ||
      /index registry not found/i.test(message) ||
      /no (search )?index/i.test(message)
    ) {
      return {
        content: [
          {
            type: 'text',
            text:
              `Full-text search is not available for map '${map}' (no search index yet). ` +
              `Use topgun_query instead for exact matching, or enable full-text search on the server.`,
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
