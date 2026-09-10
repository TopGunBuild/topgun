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
 * - `'unknown'` — a refusal kind this client version does not classify;
 *   permanent — do not retry; read {@link WriteRejection.reason}. It is what a
 *   refusal carrying no wire code, or a code from a newer server, is reported
 *   as. Reporting such a refusal under a *named* cause would tell the
 *   application something the frame does not establish — "access denied" for
 *   what may be a schema refusal — so the honest answer is that the cause is
 *   not known, not the nearest guess.
 */
export type WriteRejectionCause =
  | 'forbidden'
  | 'schema_invalid'
  | 'value_too_large'
  | 'write_lost'
  | 'stale'
  | 'unknown';

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
   * and as a React key. Always the op id — every emitter that exists today
   * refuses a specific operation, so every event has one.
   *
   * An op-less refusal would need to define its own stable id, and none is
   * defined: the only emitter that would produce one is the TODO-653 fix, which
   * has not landed. Adding that emitter means specifying its id here first.
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
  /**
   * The refused write's own hybrid-logical-clock timestamp.
   *
   * The write's clock, not the moment its refusal arrived: it is what makes
   * {@link WriteRejection.id} stable across a re-delivery of the same refusal,
   * and what lets a later write for the same key supersede this one instead of
   * leaving the record permanently marked. A refusal naming a write the client
   * no longer holds has no such clock and falls back to the observation time.
   */
  timestamp: Timestamp;
}
