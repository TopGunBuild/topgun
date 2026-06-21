/**
 * Execution-time stand-in for `@topgunbuild/client` (run tier only).
 *
 * Re-exports the real client surface verbatim, but wraps `TopGunClient` so the
 * harness can (a) prove a documented wiring snippet actually completed the WS
 * handshake against the live server, and (b) tear every client down after each
 * snippet — `close()` clears the reconnect timers that would otherwise keep the
 * Jest event loop alive (see TopGunClient.close()).
 *
 * The TYPECHECK tier is unaffected — it resolves the real package types via the
 * TS compiler (helpers/tsc.ts), not this runtime shim.
 */
// Import the REAL client via a relative path, NOT the '@topgunbuild/client'
// specifier — the jest moduleNameMapper points that specifier at THIS file, so
// importing it here would recurse infinitely.
import * as realClient from '../../../packages/client/src/index';
import type { SyncState } from '../../../packages/client/src/index';

export * from '../../../packages/client/src/index';

const RealTopGunClient = realClient.TopGunClient;

/** Every client instantiated since the last reset, for assertion + teardown. */
const tracked: InstanceType<typeof RealTopGunClient>[] = [];

// eslint-disable-next-line @typescript-eslint/no-explicit-any
class TrackedTopGunClient extends (RealTopGunClient as any) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  constructor(...args: any[]) {
    super(...args);
    tracked.push(this as unknown as InstanceType<typeof RealTopGunClient>);
  }
}

// Override the named export with the tracking subclass.
export { TrackedTopGunClient as TopGunClient };

/** Clients created since the last reset. */
export function __doctestTrackedClients(): InstanceType<typeof RealTopGunClient>[] {
  return tracked.slice();
}

/** Closes every tracked client (clearing reconnect timers) and resets tracking. */
export async function __doctestResetClients(): Promise<void> {
  const clients = tracked.splice(0, tracked.length);
  await Promise.all(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    clients.map((c) => (c as any).close?.().catch(() => {})),
  );
}

/**
 * Waits until at least one tracked client reaches CONNECTED — proof that a
 * documented `new TopGunClient({ serverUrl })` actually handshook with the live
 * server. Resolves immediately if no client opened a connection (e.g. a
 * local-only example with no serverUrl).
 */
export async function __doctestAwaitConnected(timeoutMs: number): Promise<void> {
  if (tracked.length === 0) return;
  const connectedState: SyncState = realClient.SyncState.CONNECTED;
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    for (const c of tracked) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const state = (c as any).getConnectionState?.() as SyncState | undefined;
      if (state === connectedState || state === realClient.SyncState.SYNCING) return;
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  // Surface the last seen states so a genuine handshake failure is a clear error.
  const states = tracked
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    .map((c, i) => `client[${i}]=${(c as any).getConnectionState?.()}`)
    .join(', ');
  throw new Error(`no tracked client reached CONNECTED within ${timeoutMs}ms (${states})`);
}
