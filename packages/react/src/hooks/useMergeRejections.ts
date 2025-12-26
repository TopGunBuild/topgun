import { useState, useEffect, useCallback } from 'react';
import { useClient } from './useClient';
import type { MergeRejection } from '@topgunbuild/core';

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

  const [rejections, setRejections] = useState<MergeRejection[]>([]);
  const [lastRejection, setLastRejection] = useState<MergeRejection | null>(null);

  useEffect(() => {
    const resolvers = client.getConflictResolvers();

    const unsubscribe = resolvers.onRejection((rejection) => {
      // Filter by map name if specified
      if (mapName && rejection.mapName !== mapName) {
        return;
      }

      setLastRejection(rejection);
      setRejections((prev) => {
        const next = [...prev, rejection];
        // Limit history size
        if (next.length > maxHistory) {
          return next.slice(-maxHistory);
        }
        return next;
      });
    });

    return unsubscribe;
  }, [client, mapName, maxHistory]);

  const clear = useCallback(() => {
    setRejections([]);
    setLastRejection(null);
  }, []);

  return {
    rejections,
    lastRejection,
    clear,
  };
}
