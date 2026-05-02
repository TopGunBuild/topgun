import { HLC } from '@topgunbuild/core';
import type { MergeRejection, Timestamp } from '@topgunbuild/core';
import type { OpLogEntry } from './SyncEngine';
import { SyncState } from './SyncState';

/**
 * Per-record sync-state tag exposed to applications via QueryHandle, useQuery,
 * useSyncState, and the *WithSyncState companion hooks.
 *
 * Distinct from the connection-level SyncState enum (which models the
 * SyncEngine FSM). RecordSyncState answers "has this specific record's write
 * hit the server?" rather than "is the connection alive?".
 *
 * State meanings:
 *   - `synced`     — server OP_ACK received (or no local write outstanding)
 *   - `pending`    — local write in opLog, transmission in flight (connection up)
 *   - `local-only` — local write in opLog, awaiting reconnect (connection down)
 *   - `conflicted` — server-side resolver rejected/downgraded the write
 */
export type RecordSyncState = 'synced' | 'pending' | 'conflicted' | 'local-only';

/**
 * Internal observer interface invoked at SyncEngine's existing opLog mutation
 * sites. Avoids wrapping `this.opLog` in a Proxy (rejected for hot-path perf)
 * by spelling out the integration as explicit emit calls at each mutation
 * point.
 *
 * Synchronous on purpose — call sites are already on the hot path; adding a
 * microtask queue would only add latency without buying decoupling.
 *
 * Internal-only: not re-exported from `@topgunbuild/client`'s public surface.
 */
export interface OpLogObserver {
  /**
   * Called immediately after a new entry is pushed to opLog (synced=false).
   * Fires for both fresh appends in `recordOperation` and restored pending
   * ops in `loadOpLog`.
   */
  onAppend(entry: OpLogEntry): void;
  /**
   * Called when an entry's synced flag flips true (server OP_ACK received).
   * Fires per-entry inside the per-result and per-batch forEach loops in
   * `handleOpAck`.
   */
  onAcknowledge(entry: OpLogEntry): void;
}

/**
 * The set of connection states during which a `synced=false` opLog entry
 * projects to `pending` (as opposed to `local-only`). Anything outside this
 * set means we cannot currently reach the server, so the entry is local-only.
 */
const ONLINE_STATES: ReadonlySet<SyncState> = new Set([
  SyncState.CONNECTED,
  SyncState.SYNCING,
]);

/**
 * Per-`(mapName, key)` tracker entry. Stores only the latest observed
 * timestamp (single-slot, not history) — sufficient for the four-state
 * projection and bounds memory.
 */
interface TrackerEntry {
  /** Latest observed OpLogEntry.timestamp for this (mapName, key); null if no opLog entry observed yet. */
  latestOpTimestamp: Timestamp | null;
  /** True when the latest observed opLog entry's synced flag is true. */
  syncedFlag: boolean;
  /** Latest MergeRejection seen for this (mapName, key); null if none. */
  rejection: MergeRejection | null;
}

type ChangeListener = (snapshot: ReadonlyMap<string, RecordSyncState>) => void;

/**
 * Projects existing client-observable signals (OpLog mutations, connection
 * state, MergeRejection stream) into a per-record `RecordSyncState` map per
 * map name.
 *
 * Owned by SyncEngine; SyncEngine.close() must call dispose() as part of its
 * teardown sequence.
 */
export class RecordSyncStateTracker {
  // mapName -> key -> TrackerEntry. Outer Map is created lazily per mapName.
  private readonly entries: Map<string, Map<string, TrackerEntry>> = new Map();
  // mapName -> set of change listeners. Inner emission only fires when the
  // map's projected snapshot actually changed.
  private readonly listeners: Map<string, Set<ChangeListener>> = new Map();
  // Cached snapshots per mapName. Identity changes only when contents change.
  private readonly snapshots: Map<string, ReadonlyMap<string, RecordSyncState>> = new Map();

  private connectionState: SyncState;
  private disposed = false;

  // Subscriber teardowns wired in at construction.
  private readonly disposers: Array<() => void> = [];

  constructor(initialConnectionState: SyncState = SyncState.INITIAL) {
    this.connectionState = initialConnectionState;
  }

  /**
   * Register a teardown function to be invoked on dispose(). SyncEngine wires
   * SyncStateMachine.onStateChange and ConflictResolverClient.onRejection
   * teardowns in via this.
   */
  registerDisposer(fn: () => void): void {
    if (this.disposed) {
      try {
        fn();
      } catch {
        // Defensive: swallow teardown errors during disposed registration.
      }
      return;
    }
    this.disposers.push(fn);
  }

  /**
   * OpLogObserver — called when a new entry is appended to opLog (synced=false).
   * Covers both fresh writes (recordOperation) and restored pending ops
   * (loadOpLog).
   */
  onAppend(entry: OpLogEntry): void {
    if (this.disposed) return;
    const slot = this.ensureEntry(entry.mapName, entry.key);
    // Fresh append always supersedes prior single-slot timestamp for this key.
    slot.latestOpTimestamp = entry.timestamp;
    slot.syncedFlag = entry.synced === true;
    // Late-arrival case: a prior rejection's timestamp may now be strictly
    // less than this new write — the projection rule re-evaluates on demand,
    // so we do NOT clear `slot.rejection` here. The projection compares HLC
    // values at read time and will fall through to rule 2/3 when appropriate.
    this.notifyMapChange(entry.mapName, entry.key);
  }

  /**
   * OpLogObserver — called when an existing opLog entry's synced flag flips
   * to true (server OP_ACK received). Fires per-entry inside the per-result
   * and per-batch forEach loops in handleOpAck.
   */
  onAcknowledge(entry: OpLogEntry): void {
    if (this.disposed) return;
    const inner = this.entries.get(entry.mapName);
    const slot = inner?.get(entry.key);
    if (!slot) {
      // No prior tracking — likely the ack predates the tracker's lifetime
      // (shouldn't normally happen since onAppend fires before onAcknowledge).
      // Create a slot reflecting the ack so future projections are correct.
      const created = this.ensureEntry(entry.mapName, entry.key);
      created.latestOpTimestamp = entry.timestamp;
      created.syncedFlag = true;
      this.notifyMapChange(entry.mapName, entry.key);
      return;
    }
    // Only flip when this ack matches the tracker's latest observed timestamp
    // for the key. If a newer write has been pushed after this op, the slot
    // reflects that newer write — we don't downgrade it.
    if (slot.latestOpTimestamp && HLC.compare(entry.timestamp, slot.latestOpTimestamp) === 0) {
      slot.syncedFlag = true;
      // A successful subsequent ack clears `conflicted` per spec §2:
      // "Conflicted state clears when a subsequent write for the same
      // (mapName, key) lands and is acknowledged."
      slot.rejection = null;
      this.notifyMapChange(entry.mapName, entry.key);
    }
  }

  /**
   * Subscribed by SyncEngine to ConflictResolverClient.onRejection. Marks the
   * (mapName, key) slot with the rejection; projection rule 1 decides whether
   * the late-arrival case (rejection older than latest local write) suppresses
   * the conflicted state.
   */
  onRejection(rejection: MergeRejection): void {
    if (this.disposed) return;
    const slot = this.ensureEntry(rejection.mapName, rejection.key);
    slot.rejection = rejection;
    this.notifyMapChange(rejection.mapName, rejection.key);
  }

  /**
   * Subscribed by SyncEngine to SyncStateMachine.onStateChange. Re-projects
   * `pending` <-> `local-only` for all tracked keys whose underlying opLog
   * entry is still synced=false.
   */
  onConnectionStateChange(newState: SyncState): void {
    if (this.disposed) return;
    const wasOnline = ONLINE_STATES.has(this.connectionState);
    const nowOnline = ONLINE_STATES.has(newState);
    this.connectionState = newState;
    if (wasOnline === nowOnline) {
      // Projection unchanged — only `pending` <-> `local-only` toggle is
      // affected by online/offline transitions. Skip the snapshot rebuild.
      return;
    }
    // The projection of every map with at least one un-synced entry may
    // change. Re-emit per affected map.
    for (const mapName of this.entries.keys()) {
      this.notifyMapChange(mapName, null);
    }
  }

  /**
   * Project the four-state rule for a single (mapName, key). Returns
   * 'synced' as the default for unknown keys.
   */
  get(mapName: string, key: string): RecordSyncState {
    const slot = this.entries.get(mapName)?.get(key);
    if (!slot) return 'synced';
    // Rule 1: rejection wins on >= comparison vs latest opLog timestamp.
    // Late-arrival case: if a newer local write supersedes the rejection's
    // timestamp, the rejection is retained for diagnostics but does NOT mark
    // the entry conflicted — fall through to rules 2/3.
    if (slot.rejection) {
      if (slot.latestOpTimestamp == null) {
        // Rejection observed but no local opLog entry tracked — server is the
        // merge authority. Treat as conflicted.
        return 'conflicted';
      }
      const cmp = HLC.compare(slot.rejection.timestamp, slot.latestOpTimestamp);
      if (cmp >= 0) {
        return 'conflicted';
      }
      // cmp < 0: rejection is older than the latest local write — fall through.
    }
    if (slot.latestOpTimestamp != null && slot.syncedFlag === false) {
      return ONLINE_STATES.has(this.connectionState) ? 'pending' : 'local-only';
    }
    // Either no opLog entry observed (rejection-only with cmp < 0 — rare) or
    // the latest opLog entry is synced=true.
    return 'synced';
  }

  /**
   * Snapshot of all tracked keys for a map name. Identity is stable across
   * calls until the projection for at least one key changes.
   */
  getMapSnapshot(mapName: string): ReadonlyMap<string, RecordSyncState> {
    const cached = this.snapshots.get(mapName);
    if (cached) return cached;
    const fresh = this.buildSnapshot(mapName);
    this.snapshots.set(mapName, fresh);
    return fresh;
  }

  /**
   * Subscribe to projection changes for a single map name. Listener is
   * invoked with a fresh ReadonlyMap snapshot whose identity differs from any
   * prior emission. Returns an unsubscribe function.
   */
  onChange(mapName: string, cb: ChangeListener): () => void {
    let bucket = this.listeners.get(mapName);
    if (!bucket) {
      bucket = new Set();
      this.listeners.set(mapName, bucket);
    }
    bucket.add(cb);
    return () => {
      bucket?.delete(cb);
      if (bucket && bucket.size === 0) {
        this.listeners.delete(mapName);
      }
    };
  }

  /**
   * Tear down all subscriptions and clear internal state. Idempotent.
   */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const fn of this.disposers) {
      try {
        fn();
      } catch {
        // Defensive: continue tearing down other subscriptions even if one
        // teardown throws.
      }
    }
    this.disposers.length = 0;
    this.entries.clear();
    this.listeners.clear();
    this.snapshots.clear();
  }

  // --- Internal helpers -----------------------------------------------------

  private ensureEntry(mapName: string, key: string): TrackerEntry {
    let inner = this.entries.get(mapName);
    if (!inner) {
      inner = new Map();
      this.entries.set(mapName, inner);
    }
    let slot = inner.get(key);
    if (!slot) {
      slot = { latestOpTimestamp: null, syncedFlag: false, rejection: null };
      inner.set(key, slot);
    }
    return slot;
  }

  /**
   * Recompute the snapshot for a map and emit to listeners only when at least
   * one key's projection actually changed (Map identity differs by content).
   *
   * `changedKey` is informational — when null (e.g. connection state flip),
   * we still rebuild the whole snapshot since multiple keys may have flipped
   * their projection in lockstep.
   */
  private notifyMapChange(mapName: string, changedKey: string | null): void {
    const fresh = this.buildSnapshot(mapName);
    const prev = this.snapshots.get(mapName);
    if (prev && this.snapshotsEqual(prev, fresh)) {
      // No-op emission suppressed: avoids re-rendering downstream consumers
      // when the changed key didn't actually change projection (e.g. an
      // ack on an already-synced entry).
      void changedKey;
      return;
    }
    this.snapshots.set(mapName, fresh);
    const bucket = this.listeners.get(mapName);
    if (!bucket || bucket.size === 0) return;
    for (const listener of bucket) {
      try {
        listener(fresh);
      } catch {
        // Defensive: keep other listeners running.
      }
    }
  }

  private buildSnapshot(mapName: string): ReadonlyMap<string, RecordSyncState> {
    const out = new Map<string, RecordSyncState>();
    const inner = this.entries.get(mapName);
    if (!inner) return out;
    for (const [key] of inner) {
      out.set(key, this.get(mapName, key));
    }
    return out;
  }

  private snapshotsEqual(
    a: ReadonlyMap<string, RecordSyncState>,
    b: ReadonlyMap<string, RecordSyncState>,
  ): boolean {
    if (a.size !== b.size) return false;
    for (const [k, v] of a) {
      if (b.get(k) !== v) return false;
    }
    return true;
  }
}
