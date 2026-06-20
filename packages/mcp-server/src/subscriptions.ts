/**
 * Persistent change-feed subscriptions for the MCP server.
 *
 * WHY A POLL CONTRACT — the MCP protocol is request/response: a single
 * `tools/call` cannot stream incremental results into the agent's turn. The old
 * `topgun_subscribe` worked around this by BLOCKING the tool for the whole
 * timeout (≤60 s) and then dumping the full result set as `UPDATE` "changes",
 * which mis-reported every snapshot row as a change and made removals invisible.
 *
 * The honest design for a request/response transport is an explicit poll cursor:
 * `start` opens a long-lived live query in the MCP process (returning at once),
 * the genuine per-record deltas it receives buffer here, and `poll` drains them.
 * Deltas are computed by the client's `ChangeTracker` (via `QueryHandle.onDelta`),
 * so each one carries a real `add | update | remove` type and removals are
 * first-class — never a row that silently vanishes from a re-emitted snapshot.
 */

import { randomUUID } from 'node:crypto';

import type { ChangeEvent, QueryHandle, TopGunClient } from '@topgunbuild/client';

/** Upper bound on buffered deltas per subscription; oldest are dropped first. */
export const MAX_BUFFERED_DELTAS = 1000;

/** Maximum number of concurrently active subscriptions per MCP process. */
export const MAX_ACTIVE_SUBSCRIPTIONS = 50;

/**
 * How long to wait for the server's first authoritative QUERY_RESP (the baseline
 * snapshot) before declaring the subscription un-startable. Mirrors the mutate
 * tool's confirm timeout — a subscription we cannot baseline against the server
 * is reported as an error rather than silently feeding the agent local-only data.
 */
export const SUBSCRIBE_SETTLE_TIMEOUT_MS = 5000;

export interface DeltaRecord {
  /** 'add' = entered the result set, 'update' = changed, 'remove' = left it. */
  type: ChangeEvent<Record<string, unknown>>['type'];
  key: string;
  value?: Record<string, unknown>;
  /** Wall-clock time the delta was observed by the MCP process. */
  at: string;
}

export interface ActiveSubscription {
  id: string;
  map: string;
  filter: Record<string, unknown>;
  createdAt: string;
  /** Genuine post-baseline deltas awaiting drain by `poll`. */
  buffer: DeltaRecord[];
  /** Count of deltas evicted because the buffer hit MAX_BUFFERED_DELTAS. */
  dropped: number;
  /** Total deltas observed since `start` (for the agent's situational awareness). */
  totalObserved: number;
}

interface InternalSubscription extends ActiveSubscription {
  handle: QueryHandle<Record<string, unknown>>;
  /** Set true only after the server baseline settles, so the initial snapshot's
   *  add-deltas are never reported as "changes". */
  recording: boolean;
  offDelta: () => void;
  unsubscribe: () => void;
  expiryTimer: ReturnType<typeof setTimeout> | null;
}

function nowIso(): string {
  return new Date().toISOString();
}

/**
 * Owns the live-query handles backing `topgun_subscribe`. One instance per
 * `TopGunMCPServer`; torn down with the server so no handle or timer leaks.
 */
export class SubscriptionRegistry {
  private readonly subs = new Map<string, InternalSubscription>();

  constructor(private readonly client: TopGunClient) {}

  get size(): number {
    return this.subs.size;
  }

  atCapacity(): boolean {
    return this.subs.size >= MAX_ACTIVE_SUBSCRIPTIONS;
  }

  /**
   * Open a live query, wait for the server to deliver its authoritative baseline,
   * then begin recording genuine deltas. Resolves to the subscription, or `null`
   * if the server never settled the query within the timeout (offline /
   * unreachable) — in which case nothing is registered and no handle leaks.
   */
  async start(
    map: string,
    filter: Record<string, unknown>,
    ttlMs: number,
  ): Promise<ActiveSubscription | null> {
    const id = randomUUID();
    const handle = this.client.query<Record<string, unknown>>(map, filter);

    const sub: InternalSubscription = {
      id,
      map,
      filter,
      createdAt: nowIso(),
      buffer: [],
      dropped: 0,
      totalObserved: 0,
      handle,
      recording: false,
      offDelta: () => {},
      unsubscribe: () => {},
      expiryTimer: null,
    };

    // Record deltas only once the baseline has settled. The initial snapshot is
    // delivered as a burst of `add` deltas BEFORE `recording` flips, so it is
    // intentionally ignored — only changes that happen AFTER the baseline are
    // real changes for the agent.
    sub.offDelta = handle.onDelta((changes) => {
      if (!sub.recording) return;
      for (const change of changes) {
        sub.totalObserved += 1;
        if (sub.buffer.length >= MAX_BUFFERED_DELTAS) {
          sub.buffer.shift();
          sub.dropped += 1;
        }
        sub.buffer.push({
          type: change.type,
          key: change.key,
          value: change.value,
          at: nowIso(),
        });
      }
    });

    // A no-op result listener is what actually activates the server-side live
    // query (onDelta alone does not). We drive the feed off the deltas, not the
    // full result set, so the callback body is intentionally empty.
    sub.unsubscribe = handle.subscribe(() => {});

    const settled = await this.waitForBaseline(handle, SUBSCRIBE_SETTLE_TIMEOUT_MS);
    if (!settled) {
      // Could not establish a server baseline — do not register a half-open
      // subscription that would feed the agent local-only or empty data.
      sub.offDelta();
      sub.unsubscribe();
      return null;
    }

    sub.recording = true;
    this.subs.set(id, sub);
    this.resetExpiry(sub, ttlMs);
    return sub;
  }

  /**
   * Drain and clear the buffered deltas for a subscription, refreshing its idle
   * expiry. Returns `null` if the id is unknown (stopped or expired).
   */
  poll(
    id: string,
    ttlMs: number,
  ): { deltas: DeltaRecord[]; dropped: number; sub: ActiveSubscription } | null {
    const sub = this.subs.get(id);
    if (!sub) return null;
    const deltas = sub.buffer;
    const dropped = sub.dropped;
    sub.buffer = [];
    sub.dropped = 0;
    this.resetExpiry(sub, ttlMs);
    return { deltas, dropped, sub };
  }

  /** Tear down one subscription. Returns false if the id was already gone. */
  stop(id: string): boolean {
    const sub = this.subs.get(id);
    if (!sub) return false;
    this.teardown(sub);
    this.subs.delete(id);
    return true;
  }

  /** Snapshot of the active subscriptions (no internal handles exposed). */
  list(): ActiveSubscription[] {
    return Array.from(this.subs.values()).map((s) => ({
      id: s.id,
      map: s.map,
      filter: s.filter,
      createdAt: s.createdAt,
      buffer: s.buffer,
      dropped: s.dropped,
      totalObserved: s.totalObserved,
    }));
  }

  /** Tear down every subscription. Called on server stop so nothing leaks. */
  teardownAll(): void {
    for (const sub of this.subs.values()) {
      this.teardown(sub);
    }
    this.subs.clear();
  }

  private async waitForBaseline(
    handle: QueryHandle<Record<string, unknown>>,
    timeoutMs: number,
  ): Promise<boolean> {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const timeout = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, timeoutMs);
    });
    try {
      await Promise.race([handle.whenSettled(), timeout]);
    } finally {
      if (timer) clearTimeout(timer);
    }
    return handle.isSettled;
  }

  private resetExpiry(sub: InternalSubscription, ttlMs: number): void {
    if (sub.expiryTimer) clearTimeout(sub.expiryTimer);
    const timer = setTimeout(() => this.stop(sub.id), ttlMs);
    // Do not keep the Node process alive solely for a forgotten subscription.
    if (typeof timer.unref === 'function') timer.unref();
    sub.expiryTimer = timer;
  }

  private teardown(sub: InternalSubscription): void {
    if (sub.expiryTimer) {
      clearTimeout(sub.expiryTimer);
      sub.expiryTimer = null;
    }
    try {
      sub.offDelta();
    } catch {
      // listener removal must never throw out of teardown
    }
    try {
      sub.unsubscribe();
    } catch {
      // unsubscribe must never throw out of teardown
    }
  }
}
