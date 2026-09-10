import type { WriteRejection, WriteRejectionCause } from '@topgunbuild/core';
import { logger } from './utils/logger';

/** Callback invoked once for every write the server permanently refused. */
export type WriteRejectionListener = (rejection: WriteRejection) => void;

/**
 * The server's wire codes for the refusals it attributes to a single operation,
 * mapped onto the client's closed cause vocabulary.
 *
 * These three are the whole attributable set: only a permanent refusal that the
 * server can name an operation for is sent as a per-op refusal, and that is
 * exactly `Forbidden` (403), `ValueTooLarge` (413) and `SchemaInvalid` (422).
 * Everything else the server can fail with is either transient — in which case
 * the operation stays pending and is retried, never reaching this map — or a
 * batch-level error that names no operation.
 */
const CAUSE_BY_WIRE_CODE: Readonly<Record<number, WriteRejectionCause>> = {
  403: 'forbidden',
  413: 'value_too_large',
  422: 'schema_invalid',
};

/**
 * Translate the refusal's machine-readable wire code into a
 * {@link WriteRejectionCause}.
 *
 * An unrecognised or absent code is reported as `'unknown'` rather than dropped:
 * the code is optional on the wire for historical reasons, and a refusal whose
 * code this client does not recognise is still a refusal the application must be
 * told about — swallowing it would recreate exactly the silence this surface
 * exists to remove. It is reported as `'unknown'` rather than as the nearest
 * named cause because a named cause asserts something the frame does not
 * establish: an application branching on `'forbidden'` would show "access
 * denied" for what may be a schema refusal from a newer server. The server's own
 * {@link WriteRejection.reason} carries the detail the cause cannot. The
 * unmapped code is logged, so a server that starts attributing a new refusal
 * kind is visible to operators before any consumer has to guess.
 */
export function writeRejectionCauseFromCode(code?: number): WriteRejectionCause {
  if (code !== undefined) {
    const known = CAUSE_BY_WIRE_CODE[code];
    if (known) return known;
    logger.warn(
      { code },
      'Refusal carries a wire code this client does not map to a cause — reporting it as unknown',
    );
  }
  return 'unknown';
}

/**
 * Fan-out point for {@link WriteRejection} events.
 *
 * **Holds no event buffer, by design.** The engine emits and does not store: a
 * refusal is presented, never rolled back, so the refused value stays in the
 * local map and the op log remains the system of record. Truncating or dropping
 * a notification therefore never destroys data, and memory is O(listeners)
 * rather than O(events) — which is what keeps a reconnect burst of thousands of
 * refusals bounded on the client. Applications that want history ask for it
 * explicitly (`useWriteRejections`'s bounded `maxHistory` ring).
 *
 * Events are session-scoped and do not survive a page reload; after one, a
 * refused record reads as synced while holding a value the server rejected. The
 * durable refused-writes store that would close this is tracked as TODO-667.
 */
export class WriteRejectionEmitter {
  private readonly listeners = new Set<WriteRejectionListener>();

  /**
   * Register `listener` and return its unsubscribe function. Calling the
   * returned function twice is harmless.
   */
  subscribe(listener: WriteRejectionListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Deliver `rejection` to every listener, synchronously.
   *
   * Each listener is called inside its own `try`/`catch`: a listener that throws
   * is logged and the fan-out continues, so one badly-behaved subscriber can
   * neither hide the refusal from the others nor abort the message handler this
   * runs inside.
   */
  emit(rejection: WriteRejection): void {
    for (const listener of this.listeners) {
      try {
        listener(rejection);
      } catch (err) {
        logger.error({ err, id: rejection.id }, 'Write-rejection listener threw');
      }
    }
  }

  /** How many listeners are currently registered. */
  get listenerCount(): number {
    return this.listeners.size;
  }

  /** Drop every listener. Called when the engine closes. */
  clear(): void {
    this.listeners.clear();
  }
}
