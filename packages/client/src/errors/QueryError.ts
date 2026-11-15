/**
 * Machine-distinguishable reason a query handle was refused.
 *
 * - `QUERY_LIVE_AGGREGATE_UNSUPPORTED`: a live query (`client.query()`, `useQuery`)
 *   carried a non-empty `groupBy` or `aggregations`. A live query receives
 *   per-record changes, which cannot keep grouped rows correct; run the query
 *   with `queryOnce` / `queryOncePaged` instead.
 */
export type QueryErrorCode = 'QUERY_LIVE_AGGREGATE_UNSUPPORTED';

/**
 * Thrown synchronously when a query handle is built with options it cannot
 * serve. Nothing is registered and nothing is sent to the server.
 */
export class QueryError extends Error {
  public readonly name = 'QueryError';
  public readonly code: QueryErrorCode;
  public readonly mapName: string;

  constructor(message: string, code: QueryErrorCode, mapName: string) {
    super(message);
    this.code = code;
    this.mapName = mapName;

    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, QueryError);
    }
  }
}
