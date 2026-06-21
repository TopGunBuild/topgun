import * as React from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * `useSyncExternalStore` accessor that degrades gracefully on React < 18.
 *
 * React 18 added `useSyncExternalStore` as a named export. The package's
 * peer range is `react >= 16.8`, so we cannot assume the export exists.
 * When it does, we use it directly — it returns the cached snapshot
 * synchronously during render (no first-paint loading flash) and re-checks
 * the snapshot immediately after subscribing (closing the
 * subscribe-in-passive-effect gap), and is tear-free under concurrent
 * rendering. When it does not (React 16.8 / 17), we fall back to a
 * `useState` + `useEffect(subscribe)` shim that preserves the same call
 * signature; the synchronous-snapshot and tearing guarantees simply reduce
 * to the legacy behaviour on those older runtimes.
 *
 * The fallback intentionally mirrors React's own
 * `use-sync-external-store/shim` semantics closely enough for our hooks: it
 * seeds state from `getSnapshot()` (so the first render still reads the
 * cached value), re-reads after subscribing, and re-reads on every store
 * notification.
 */

type Subscribe = (onStoreChange: () => void) => () => void;

// React 18 exposes useSyncExternalStore as a named export; older versions do not.
const nativeUSES = (React as { useSyncExternalStore?: <T>(s: Subscribe, g: () => T) => T })
  .useSyncExternalStore;

function fallbackUseSyncExternalStore<T>(subscribe: Subscribe, getSnapshot: () => T): T {
  // Seed from the snapshot so the first render reflects the cached value,
  // matching useSyncExternalStore's synchronous-read contract as closely as
  // a passive-effect shim can.
  const [snapshot, setSnapshot] = useState<T>(getSnapshot);

  // Keep the latest getSnapshot in a ref so the subscribe effect does not
  // need it as a dependency (subscribe identity drives re-subscription).
  const getSnapshotRef = useRef(getSnapshot);
  getSnapshotRef.current = getSnapshot;

  useEffect(() => {
    let mounted = true;
    const checkForUpdates = () => {
      if (!mounted) return;
      const next = getSnapshotRef.current();
      // Functional update so we compare against the freshest committed value
      // and avoid scheduling a render when the snapshot is referentially equal.
      setSnapshot((prev) => (Object.is(prev, next) ? prev : next));
    };
    const unsubscribe = subscribe(checkForUpdates);
    // Re-check immediately after subscribing to catch a mutation that landed
    // between the initial getSnapshot() seed and the subscription taking effect.
    checkForUpdates();
    return () => {
      mounted = false;
      unsubscribe();
    };
  }, [subscribe]);

  return snapshot;
}

/**
 * Subscribe to an external store and read a synchronous, referentially-stable
 * snapshot. `subscribe` MUST be a stable function (memoize with `useCallback`
 * keyed by the store identity) and `getSnapshot` MUST return a cached value
 * that only changes identity when the store actually changes — otherwise React
 * re-subscribes / re-renders on every pass.
 */
export function useExternalStore<T>(subscribe: Subscribe, getSnapshot: () => T): T {
  const impl = nativeUSES ?? fallbackUseSyncExternalStore;
  return impl(subscribe, getSnapshot);
}

/**
 * A hook-local external store for hooks whose value is produced imperatively
 * (e.g. handle lifecycle managed across effects, debounce timers) rather than
 * read directly from a long-lived client store. The hook keeps the current
 * snapshot in `ref.current` and calls `notify()` after mutating it; React reads
 * the snapshot through `useSyncExternalStore`, giving tearing-safety and unmount
 * safety without an `isMounted` ref.
 *
 * Returns `[value, notify]` where `value` is the React-tracked snapshot and
 * `notify` schedules a re-render after the caller mutates `ref.current`.
 * `getSnapshot` MUST return a referentially-stable value between notifies.
 */
export function useLocalStore<T>(getSnapshot: () => T): [T, () => void] {
  const listenersRef = useRef(new Set<() => void>());

  const subscribe = useCallback<Subscribe>((onStoreChange) => {
    const listeners = listenersRef.current;
    listeners.add(onStoreChange);
    return () => {
      listeners.delete(onStoreChange);
    };
  }, []);

  const notify = useCallback(() => {
    for (const l of listenersRef.current) l();
  }, []);

  const value = useExternalStore(subscribe, getSnapshot);
  return [value, notify];
}

/**
 * Subscribe to a store that notifies a bare `() => void` callback (the common
 * shape of TopGun's CRDT maps, counters, and trackers) and drive re-renders via
 * a monotonically increasing **version counter**. The returned number is a
 * stable primitive that only changes when the store notifies, so it satisfies
 * the `getSnapshot` referential-stability contract while the caller continues
 * to return the live store object from the hook.
 *
 * `subscribeToStore(onChange)` must register `onChange` with the store and
 * return an unsubscribe function. It must be stable (memoize by store identity).
 */
export function useStoreVersion(subscribeToStore: Subscribe): number {
  // A mutable cell holding the current version. Bumped inside the store's
  // notify callback (before React's onStoreChange) so getSnapshot reads the
  // new value synchronously. Never recreated, so its identity is stable.
  const versionRef = useRef(0);

  const subscribe = useCallback<Subscribe>(
    (onStoreChange) => {
      return subscribeToStore(() => {
        versionRef.current += 1;
        onStoreChange();
      });
    },
    [subscribeToStore],
  );

  const getSnapshot = useCallback(() => versionRef.current, []);

  return useExternalStore(subscribe, getSnapshot);
}
