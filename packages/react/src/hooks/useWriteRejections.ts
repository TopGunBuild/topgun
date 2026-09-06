import { useCallback, useEffect, useRef } from 'react';
import { useClient } from './useClient';
import { useLocalStore } from './internal/useExternalStore';
import type { WriteRejection } from '@topgunbuild/core';

const EMPTY_REJECTIONS: WriteRejection[] = [];

/**
 * Options for useWriteRejections hook.
 */
export interface UseWriteRejectionsOptions {
  /** Only report refusals for this map (optional). */
  mapName?: string;

  /** Maximum number of refusals to keep in history. Defaults to 100. */
  maxHistory?: number;
}

/**
 * Result type for useWriteRejections hook.
 */
export interface UseWriteRejectionsResult {
  /** Recent refusals, oldest first, at most `maxHistory` of them. */
  rejections: WriteRejection[];

  /** The most recent refusal, or `null` when there is none. */
  lastRejection: WriteRejection | null;

  /** Forget every refusal collected so far. */
  clear: () => void;
}

/**
 * React hook for writes the server **permanently refused** — a permission
 * denial, a schema violation, an oversized value.
 *
 * A refused write is never rolled back: the local value stays, and this hook is
 * how the UI learns the server will not accept it. Retrying cannot help, so the
 * useful responses are to tell the user, or to write a different value.
 *
 * **These events are session-scoped and do NOT survive a page reload.** After a
 * reload the list starts empty and the refused record reads as `'synced'` again,
 * so a `'synced'` state does not by itself guarantee the server accepted the
 * write. The durable refused-writes store that would close this gap is tracked
 * as TODO-667.
 *
 * Distinct from `useMergeRejections`, which reports the *merge* outcome of a
 * write the server did accept for consideration. `useSyncState` shows the same
 * records as `'conflicted'` without any further integration.
 *
 * Bounded on purpose: history is capped at `maxHistory` and refusals are
 * deduplicated by event id, so a reconnect that refuses a whole offline queue
 * costs one re-render per batch and O(maxHistory) memory, not one per refusal.
 *
 * @param options Optional filtering and configuration
 * @returns Refusal list and utilities
 *
 * @example Tell the user a write was refused
 * ```tsx
 * function TodoList() {
 *   const { lastRejection, clear } = useWriteRejections({ mapName: 'todos' });
 *
 *   useEffect(() => {
 *     if (!lastRejection) return;
 *     toast.error(
 *       lastRejection.cause === 'forbidden'
 *         ? `You cannot edit ${lastRejection.key}`
 *         : lastRejection.reason,
 *     );
 *     clear();
 *   }, [lastRejection, clear]);
 *
 *   return <ul>...</ul>;
 * }
 * ```
 *
 * @example Review everything refused while offline
 * ```tsx
 * function RefusedWrites() {
 *   const { rejections } = useWriteRejections({ maxHistory: 50 });
 *
 *   return (
 *     <ul>
 *       {rejections.map((r) => (
 *         <li key={r.id}>
 *           {r.mapName}/{r.key}: {r.reason} ({r.cause})
 *         </li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 */
export function useWriteRejections(
  options: UseWriteRejectionsOptions = {},
): UseWriteRejectionsResult {
  const client = useClient();
  const { mapName, maxHistory = 100 } = options;

  // History accumulates in a ref read through useSyncExternalStore (tearing-/
  // unmount-safe). The ref holds a referentially-stable array between notifies.
  const rejectionsRef = useRef<WriteRejection[]>(EMPTY_REJECTIONS);
  const getRejectionsSnapshot = useCallback(() => rejectionsRef.current, []);
  const [rejections, notifyRejections] = useLocalStore(getRejectionsSnapshot);

  // Ids already taken. Rebuilt from the retained history on every drain, so it
  // is bounded by `maxHistory` rather than by the number of refusals seen:
  // dedup protects the window the hook can still show, which is the window a
  // double toast could be raised from.
  const seenIdsRef = useRef<Set<string>>(new Set());

  // Refusals that have arrived but not yet been published to React. A reconnect
  // that refuses a whole offline queue delivers thousands of events in one turn;
  // draining them on a microtask turns that into ONE notify.
  const pendingRef = useRef<WriteRejection[]>([]);
  const drainScheduledRef = useRef(false);

  useEffect(() => {
    let active = true;

    const drain = () => {
      drainScheduledRef.current = false;
      const batch = pendingRef.current;
      pendingRef.current = [];
      if (!active || batch.length === 0) return;

      const merged = [...rejectionsRef.current, ...batch];
      const next = merged.length > maxHistory ? merged.slice(-maxHistory) : merged;
      rejectionsRef.current = next;
      seenIdsRef.current = new Set(next.map((r) => r.id));
      notifyRejections();
    };

    const unsubscribe = client.onWriteRejected((rejection) => {
      if (mapName && rejection.mapName !== mapName) return;
      // Same refusal delivered twice must not toast twice.
      if (seenIdsRef.current.has(rejection.id)) return;
      seenIdsRef.current.add(rejection.id);
      pendingRef.current.push(rejection);
      if (drainScheduledRef.current) return;
      drainScheduledRef.current = true;
      queueMicrotask(drain);
    });

    return () => {
      active = false;
      unsubscribe();
      pendingRef.current = [];
    };
  }, [client, mapName, maxHistory, notifyRejections]);

  const clear = useCallback(() => {
    rejectionsRef.current = EMPTY_REJECTIONS;
    seenIdsRef.current = new Set();
    pendingRef.current = [];
    notifyRejections();
  }, [notifyRejections]);

  return {
    rejections,
    // Derived, not a second piece of state: the newest entry of a list that
    // only changes identity on a drain. One store notify, one render.
    lastRejection: rejections.length > 0 ? rejections[rejections.length - 1] : null,
    clear,
  };
}
