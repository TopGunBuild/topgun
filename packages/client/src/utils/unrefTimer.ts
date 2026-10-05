/**
 * Detach a background timer from the host event loop, and hand it back.
 *
 * The rule for this package: a timer that nobody is awaiting — a heartbeat, a
 * poll, a reconnect backoff, a grace window — must never be the sole reason a
 * Node process (or a Jest worker) stays alive. A client that is built and then
 * abandoned without close() has to let the process exit.
 *
 * A timer that bounds a call the application IS awaiting (a request timeout) is
 * the opposite case and stays ref'd: unref'ing it would let the process exit
 * with the caller's promise still pending, so the timeout would never be
 * reported.
 *
 * Node timers expose unref(); in browsers setTimeout returns a number with no
 * unref(), so this is a no-op there.
 */
export function unrefTimer<T extends ReturnType<typeof setTimeout> | null>(timer: T): T {
  (timer as unknown as { unref?: () => void } | null)?.unref?.();
  return timer;
}
