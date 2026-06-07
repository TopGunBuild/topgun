import type { QueryResultItem } from '../QueryHandle';

/**
 * Reason a one-shot queryOnce could not obtain authoritative server data.
 *
 * - `offline`: the client is not connected to the server, so no authoritative
 *   QUERY_RESP can arrive.
 * - `timeout`: the client is (or was) connected but no server QUERY_RESP
 *   settled the query within the configured timeout window.
 */
export type QueryOnceUnsettledReason = 'offline' | 'timeout';

/**
 * Thrown by `TopGunClient.queryOnce` when authoritative server data could not be
 * obtained — the client was offline or the settle wait timed out — and the
 * caller did NOT opt into local fallback (`allowLocal` unset/false).
 *
 * The contract is deliberate: queryOnce NEVER silently returns local/stale data.
 * A successful resolve always means settled server data; this rejection always
 * means "no authoritative answer was available". Callers that prefer a local
 * snapshot in this situation opt in with `{ allowLocal: true }`, which surfaces
 * the snapshot via {@link QueryOnceLocalError} instead.
 */
export class QueryOnceUnsettledError extends Error {
  public readonly name = 'QueryOnceUnsettledError';
  public readonly code = 'QUERY_ONCE_UNSETTLED';
  public readonly reason: QueryOnceUnsettledReason;

  constructor(reason: QueryOnceUnsettledReason, mapName: string) {
    super(
      reason === 'offline'
        ? `queryOnce("${mapName}") could not reach the server (offline). No authoritative ` +
            `result is available. Pass { allowLocal: true } to accept the local snapshot, ` +
            `or retry once connected.`
        : `queryOnce("${mapName}") timed out waiting for an authoritative server response. ` +
            `Increase { timeoutMs }, pass { allowLocal: true } to accept the local snapshot, ` +
            `or retry.`,
    );
    this.reason = reason;

    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, QueryOnceUnsettledError);
    }
  }
}

/**
 * Thrown by `TopGunClient.queryOnce({ allowLocal: true })` when authoritative
 * server data could not be obtained (offline or timeout) but a local snapshot is
 * available and the caller opted to accept it.
 *
 * The local snapshot is carried on {@link localData}. Surfacing the fallback as a
 * thrown, typed error (rather than a normal resolve) is what makes settled server
 * data UNAMBIGUOUSLY distinguishable from non-settled local data: a normal
 * `queryOnce` resolve is ALWAYS settled server data; a `QueryOnceLocalError`
 * catch is ALWAYS a non-settled local snapshot. There is no in-band ambiguity.
 */
export class QueryOnceLocalError<T> extends Error {
  public readonly name = 'QueryOnceLocalError';
  public readonly code = 'QUERY_ONCE_LOCAL_FALLBACK';
  public readonly reason: QueryOnceUnsettledReason;
  /** The non-settled local snapshot for the query (may be empty). */
  public readonly localData: QueryResultItem<T>[];

  constructor(reason: QueryOnceUnsettledReason, mapName: string, localData: QueryResultItem<T>[]) {
    super(
      `queryOnce("${mapName}") returned a NON-SETTLED local snapshot ` +
        `(${localData.length} row(s)) because the server was ${reason === 'offline' ? 'unreachable' : 'too slow'}. ` +
        `Read err.localData; this data is NOT authoritative.`,
    );
    this.reason = reason;
    this.localData = localData;

    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, QueryOnceLocalError);
    }
  }
}
