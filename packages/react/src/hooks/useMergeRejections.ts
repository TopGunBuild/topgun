import { useState, useEffect, useCallback, useRef } from 'react';
import { useClient } from './useClient';
import { useLocalStore } from './internal/useExternalStore';
import type { MergeRejection } from '@topgunbuild/core';

const EMPTY_REJECTIONS: MergeRejection[] = [];

/**
 * Options for useMergeRejections hook.
 */
export interface UseMergeRejectionsOptions {
  /** Filter rejections by map name (optional) */
  mapName?: string;

  /** Maximum number of rejections to keep in history */
  maxHistory?: number;
}

/**
 * Result type for useMergeRejections hook.
 */
export interface UseMergeRejectionsResult {
  /** List of recent merge rejections */
  rejections: MergeRejection[];

  /** Last rejection received */
  lastRejection: MergeRejection | null;

  /** Clear rejection history */
  clear: () => void;
}

/**
 * React hook for subscribing to merge rejection events.
 *
 * Merge rejections occur when a custom conflict resolver rejects
 * a client's write operation. This hook allows you to:
 * - Display rejection notifications to users
 * - Refresh local state after rejection
 * - Log conflicts for debugging
 *
 * @param options Optional filtering and configuration
 * @returns Rejection list and utilities
 *
 * @example Show rejection notifications
 * ```tsx
 * function BookingForm() {
 *   const { lastRejection, clear } = useMergeRejections({
 *     mapName: 'bookings'
 *   });
 *
 *   useEffect(() => {
 *     if (lastRejection) {
 *       toast.error(`Booking failed: ${lastRejection.reason}`);
 *       clear(); // Clear after showing notification
 *     }
 *   }, [lastRejection]);
 *
 *   return <form>...</form>;
 * }
 * ```
 *
 * @example Track all rejections
 * ```tsx
 * function ConflictLog() {
 *   const { rejections } = useMergeRejections({ maxHistory: 50 });
 *
 *   return (
 *     <ul>
 *       {rejections.map((r, i) => (
 *         <li key={i}>
 *           {r.mapName}/{r.key}: {r.reason}
 *         </li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 */
export function useMergeRejections(
  options: UseMergeRejectionsOptions = {},
): UseMergeRejectionsResult {
  const client = useClient();
  const { mapName, maxHistory = 100 } = options;

  // Rejections accumulate in a ref read through useSyncExternalStore (tearing-/
  // unmount-safe). The ref holds a referentially-stable array between notifies.
  const rejectionsRef = useRef<MergeRejection[]>(EMPTY_REJECTIONS);
  const getRejectionsSnapshot = useCallback(() => rejectionsRef.current, []);
  const [rejections, notifyRejections] = useLocalStore(getRejectionsSnapshot);

  const [lastRejection, setLastRejection] = useState<MergeRejection | null>(null);

  useEffect(() => {
    const resolvers = client.getConflictResolvers();

    const unsubscribe = resolvers.onRejection((rejection) => {
      // Filter by map name if specified
      if (mapName && rejection.mapName !== mapName) {
        return;
      }

      setLastRejection(rejection);
      const next = [...rejectionsRef.current, rejection];
      rejectionsRef.current = next.length > maxHistory ? next.slice(-maxHistory) : next;
      notifyRejections();
    });

    return unsubscribe;
  }, [client, mapName, maxHistory, notifyRejections]);

  const clear = useCallback(() => {
    rejectionsRef.current = EMPTY_REJECTIONS;
    notifyRejections();
    setLastRejection(null);
  }, [notifyRejections]);

  return {
    rejections,
    lastRejection,
    clear,
  };
}
