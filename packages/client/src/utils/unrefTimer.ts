/**
 * Detach a background timer from the host event loop, and hand it back.
 *
 * The rule for this package: a background timer — a heartbeat, a reconnect
 * backoff, a grace window, a periodic refresh — must never be the sole reason
 * a Node process (or a Jest worker) stays alive.
 *
 * There is exactly one exception: the timer that is the transport's only
 * carrier of liveness. An open WebSocket client holds the process through its
 * socket handle, so every timer around that socket can be unref'd. The HTTP
 * transport has no handle between polls, so its polling interval is what
 * "open" means there and stays ref'd until close() clears it
 * (HttpSyncProvider.startPolling). No other timer qualifies.
 *
 * Separately, a timer that bounds a call the application IS awaiting (a
 * request timeout) is not a background timer and also stays ref'd: unref'ing
 * it would let the process exit with the caller's promise still pending, so
 * the timeout would never be reported. Those are one-shot and bounded.
 *
 * Node timers expose unref(); in browsers setTimeout returns a number with no
 * unref(), so this is a no-op there.
 */
export function unrefTimer<T extends ReturnType<typeof setTimeout> | null>(timer: T): T {
  (timer as unknown as { unref?: () => void } | null)?.unref?.();
  return timer;
}
