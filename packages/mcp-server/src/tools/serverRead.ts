/**
 * Shared server-authoritative read helper for the read-only MCP tools.
 *
 * `topgun_schema`, `topgun_stats` (per-map), and `topgun_explain` used to iterate
 * `client.getMap(map).entries()` — the MCP process's LOCAL CRDT replica. That
 * replica is only ever populated by `topgun_mutate` writes made in the same
 * process; the one server read (`queryOncePaged`, used by `topgun_query`) never
 * hydrates it. The result was that every read tool reported "empty / 0 records"
 * for a server map holding real data — even right after a query returned its rows.
 *
 * This helper routes those tools through the same authoritative path
 * `topgun_query` uses: a one-shot, settled server read via `queryOncePaged`. A
 * normal resolve is ALWAYS settled server data; an offline/timeout is surfaced as
 * {@link ServerReadUnsettledError} so the tools can say "could not reach the
 * server — this is NOT an empty result" rather than silently answering "0".
 */

import { QueryOnceUnsettledError } from '@topgunbuild/client';
import type { ToolContext } from '../types';

/** A single server-resident record: its map key plus the record body. */
export interface ServerRecord {
  key: string;
  value: Record<string, unknown>;
}

export interface ServerReadResult {
  /** Record bodies sampled from the server (authoritative, settled). */
  records: ServerRecord[];
  /**
   * True when the server signalled that more rows exist beyond the sampled page
   * (the sample is capped at `config.maxLimit`). Callers should label counts
   * derived from the sample as a lower bound when this is set, so a large map is
   * never silently undercounted.
   */
  hasMore: boolean;
}

/**
 * Thrown when a server-authoritative read could not settle (client offline, or
 * the settle wait timed out). Read tools MUST surface this as an explicit
 * "not reachable" error and never as an empty/zero answer, so an unreachable
 * server is never confused with a genuinely empty map.
 */
export class ServerReadUnsettledError extends Error {
  public readonly name = 'ServerReadUnsettledError';
  constructor(
    public readonly reason: 'offline' | 'timeout',
    public readonly map: string,
  ) {
    super(
      reason === 'offline'
        ? `Could not read map '${map}': the server is unreachable (client offline). ` +
            `This is NOT an empty result.`
        : `Could not read map '${map}': the server read did not settle in time. ` +
            `This is NOT an empty result.`,
    );
  }
}

// queryOncePaged is a public TopGunClient method, but its overloaded generic
// signature does not narrow cleanly here; cast to the minimal shape we use —
// the same pattern handleQuery applies for the paged read.
type ClientWithPaged = {
  queryOncePaged(
    map: string,
    filter: { where?: Record<string, unknown>; limit?: number },
  ): Promise<{
    items: Array<Record<string, unknown> & { _key: string }>;
    hasMore: boolean;
  }>;
};

/**
 * Fetch a settled, server-authoritative sample of a map's records.
 *
 * The sample is bounded by `config.maxLimit` (the same ceiling user-facing
 * queries use); when the server reports more rows exist, `hasMore` is set so the
 * caller can label derived counts as a lower bound. An optional `where` filter is
 * pushed to the server; callers that need both a total and a filtered subset
 * (e.g. `explain`) should fetch unfiltered and filter the returned sample.
 */
export async function fetchServerRecords(
  ctx: ToolContext,
  map: string,
  opts: { filter?: Record<string, unknown> } = {},
): Promise<ServerReadResult> {
  const filter: { where?: Record<string, unknown>; limit: number } = {
    limit: ctx.config.maxLimit,
  };
  if (opts.filter) {
    filter.where = opts.filter;
  }

  try {
    const paged = await (ctx.client as unknown as ClientWithPaged).queryOncePaged(map, filter);
    const records: ServerRecord[] = paged.items.map((item) => {
      const { _key, ...value } = item;
      return { key: _key, value };
    });
    return { records, hasMore: paged.hasMore };
  } catch (error) {
    if (error instanceof QueryOnceUnsettledError) {
      throw new ServerReadUnsettledError(error.reason, map);
    }
    throw error;
  }
}
