/**
 * Machine-distinguishable reason a query failed.
 *
 * - `QUERY_LIVE_AGGREGATE_UNSUPPORTED`: a live query (`client.query()`, `useQuery`)
 *   carried a non-empty `groupBy` or `aggregations`. A live query receives
 *   per-record changes, which cannot keep grouped rows correct; run the query
 *   with `queryOnce` / `queryOncePaged` instead.
 * - `QUERY_SNAPSHOT_OVERFLOW`: the server could not keep every change made
 *   while it built a live query's snapshot, and the client's resubscribes
 *   (at most three per subscription) overflowed too. The rows shown may miss
 *   changes; they are kept.
 * - `QUERY_FAILED`: the server answered with an error this client does not
 *   know. The server's own code is at the start of the message.
 */
export type QueryErrorCode =
  | 'QUERY_LIVE_AGGREGATE_UNSUPPORTED'
  | 'QUERY_SNAPSHOT_OVERFLOW'
  | 'QUERY_FAILED';

/**
 * A query error. It is thrown synchronously when a query handle is built with
 * options it cannot serve (nothing is registered or sent), or delivered from a
 * server response through `QueryHandle.onError` / `getError()` and as the
 * rejection of `queryOnce`, `queryOncePaged` and `loadMore`.
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
