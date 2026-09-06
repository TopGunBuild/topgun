import type { Timestamp } from './HLC';

/**
 * Why the server refused a write.
 *
 * A closed union, deliberately not a free string: it mirrors the server's own
 * error vocabulary, so the two cannot drift into two spellings of the same
 * cause, and a consumer switching on it is told by the compiler when a new
 * cause appears.
 *
 * - `'forbidden'`, `'schema_invalid'`, `'value_too_large'` — the server's
 *   permanent refusals, decided on admission, before anything is applied.
 * - `'write_lost'` — the write was acknowledged and then lost. See
 *   {@link WriteRejection.previouslyAcked}: **no emitter produces this cause
 *   today**; it is emitted by the TODO-653 fix.
 * - `'stale'` — refused on arrival after a long offline period.
 */
export type WriteRejectionCause =
  | 'forbidden'
  | 'schema_invalid'
  | 'value_too_large'
  | 'write_lost'
  | 'stale';

/**
 * A write the server will never accept, reported to the application.
 *
 * The local value is deliberately **kept**: a refusal is presented, not rolled
 * back, so the user's data is never silently destroyed by a server decision.
 * That is what {@link WriteRejection.keptLocally} records.
 *
 * These events are session-scoped and do not survive a page reload — after a
 * reload a refused record reads as synced while holding a value the server
 * rejected. A durable refused-writes store is tracked as TODO-667.
 */
export interface WriteRejection {
  /**
   * Stable identity of this rejection: used to deduplicate repeated deliveries
   * and as a React key. The op id where the write has one, otherwise
   * `` `${mapName}:${key}:${millis}:${counter}` ``.
   */
  id: string;
  /** Map the refused write targeted. */
  mapName: string;
  /** Key the refused write targeted. */
  key: string;
  /**
   * The value the caller tried to write, snapshotted at emission — never a
   * reference into live op state, which may have moved on by the time the
   * application reads it.
   */
  attemptedValue: unknown;
  /** Machine-readable cause, for branching. */
  cause: WriteRejectionCause;
  /** Human-readable reason, server-provided where available. */
  reason: string;
  /**
   * Whether the local value was kept despite the refusal. Always `true` today:
   * no code path rolls back a locally-applied write because the server refused
   * it. The field exists so an application never has to assume it.
   */
  keptLocally: boolean;
  /**
   * Whether the server had already acknowledged this write before it was lost —
   * the difference between "your write was never accepted" and "your write was
   * accepted and then lost", which the user deserves to be told apart.
   *
   * **Shape only today: nothing emits `true`.** The emitter arrives with the
   * TODO-653 fix, together with `cause: 'write_lost'`; until then every event
   * carries `false`.
   */
  previouslyAcked: boolean;
  /** When the refusal was observed, on the client's hybrid logical clock. */
  timestamp: Timestamp;
}
