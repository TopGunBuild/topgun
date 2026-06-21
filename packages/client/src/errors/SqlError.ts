/**
 * Machine-distinguishable error code for a server SQL query failure.
 *
 * - `FEATURE_DISABLED`: the server cannot run SQL at all — it was built without
 *   the `datafusion` feature, or compiled with it but no SQL backend is wired.
 *   This is a deployment/build fact, not a problem with the query string;
 *   retrying or rewriting the SQL will not help.
 * - `undefined`: a query/execution failure (parse error, unknown table, type
 *   mismatch, …). The human-readable {@link SqlError.message} describes it.
 */
export type SqlErrorCode = 'FEATURE_DISABLED';

/**
 * Thrown by `TopGunClient.sql()` / `SyncEngine.sql()` when the server rejects a
 * SQL query.
 *
 * The {@link code} field makes the failure class machine-distinguishable without
 * string-matching the message: a caller can branch on
 * `err.code === 'FEATURE_DISABLED'` to detect a server that simply does not have
 * SQL enabled (compile-time opt-in via `--features datafusion`) versus an actual
 * query error. Crucially, a disabled server now returns this typed error
 * promptly instead of leaving the request to hang until its timeout.
 */
export class SqlError extends Error {
  public readonly name = 'SqlError';
  /** Machine-distinguishable error class; `undefined` for query/execution errors. */
  public readonly code?: SqlErrorCode | string;

  constructor(message: string, code?: SqlErrorCode | string) {
    super(message);
    this.code = code;

    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, SqlError);
    }
  }
}
