import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import type { JournalEvent, JournalEventType } from '@topgunbuild/core';
import type { EventJournalReader, JournalSubscribeOptions } from '@topgunbuild/client';
import { useClient } from './useClient';

/**
 * Options for useEventJournal hook.
 */
export interface UseEventJournalOptions {
  /** Start from specific sequence */
  fromSequence?: bigint;
  /** Filter by map name */
  mapName?: string;
  /** Filter by event types */
  types?: JournalEventType[];
  /** Maximum events to keep in state (default: 100) */
  maxEvents?: number;
  /** Called when new event is received */
  onEvent?: (event: JournalEvent) => void;
  /** Pause subscription */
  paused?: boolean;
}

/**
 * Result type for useEventJournal hook.
 */
export interface UseEventJournalResult {
  /** Array of recent events (newest last) */
  events: JournalEvent[];
  /** Last received event */
  lastEvent: JournalEvent | null;
  /** Clear accumulated events */
  clearEvents: () => void;
  /** Read historical events from sequence */
  readFrom: (sequence: bigint, limit?: number) => Promise<JournalEvent[]>;
  /** Get latest sequence number */
  getLatestSequence: () => Promise<bigint>;
  /** Whether subscription is active */
  isSubscribed: boolean;
}

/**
 * React hook for subscribing to Event Journal changes.
 *
 * The Event Journal captures all map changes (PUT, UPDATE, DELETE) as an
 * append-only log, useful for:
 * - Real-time activity feeds
 * - Audit trails
 * - Change notifications
 * - Debugging and monitoring
 *
 * @example Basic usage - show all changes
 * ```tsx
 * function ActivityFeed() {
 *   const { events, lastEvent } = useEventJournal();
 *
 *   return (
 *     <ul>
 *       {events.map((e) => (
 *         <li key={e.sequence.toString()}>
 *           {e.type} {e.mapName}:{e.key}
 *         </li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 *
 * @example Filter by map name
 * ```tsx
 * function UserActivityFeed() {
 *   const { events } = useEventJournal({ mapName: 'users' });
 *
 *   return (
 *     <ul>
 *       {events.map((e) => (
 *         <li key={e.sequence.toString()}>
 *           User {e.key}: {e.type}
 *         </li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 *
 * @example With event callback
 * ```tsx
 * function NotifyingComponent() {
 *   const { events } = useEventJournal({
 *     mapName: 'orders',
 *     types: ['PUT'],
 *     onEvent: (event) => {
 *       toast.success(`New order: ${event.key}`);
 *     },
 *   });
 *
 *   return <OrderList events={events} />;
 * }
 * ```
 */
export function useEventJournal(
  options: UseEventJournalOptions = {}
): UseEventJournalResult {
  const client = useClient();
  const [events, setEvents] = useState<JournalEvent[]>([]);
  const [lastEvent, setLastEvent] = useState<JournalEvent | null>(null);
  const [isSubscribed, setIsSubscribed] = useState(false);

  const isMounted = useRef(true);
  const journalRef = useRef<EventJournalReader | null>(null);

  const maxEvents = options.maxEvents ?? 100;

  // Store options in ref to avoid re-subscription on every render
  const optionsRef = useRef(options);
  optionsRef.current = options;

  // Clear events callback
  const clearEvents = useCallback(() => {
    setEvents([]);
    setLastEvent(null);
  }, []);

  // Read historical events
  const readFrom = useCallback(
    async (sequence: bigint, limit?: number): Promise<JournalEvent[]> => {
      if (!journalRef.current) {
        journalRef.current = client.getEventJournal();
      }
      return journalRef.current.readFrom(sequence, limit);
    },
    [client]
  );

  // Get latest sequence
  const getLatestSequence = useCallback(async (): Promise<bigint> => {
    if (!journalRef.current) {
      journalRef.current = client.getEventJournal();
    }
    return journalRef.current.getLatestSequence();
  }, [client]);

  // Serialize filter options for dependency comparison
  const filterKey = useMemo(
    () =>
      JSON.stringify({
        mapName: options.mapName,
        types: options.types,
        fromSequence: options.fromSequence?.toString(),
        paused: options.paused,
      }),
    [options.mapName, options.types, options.fromSequence, options.paused]
  );

  useEffect(() => {
    isMounted.current = true;

    // Don't subscribe if paused
    if (options.paused) {
      setIsSubscribed(false);
      return;
    }

    const journal = client.getEventJournal();
    journalRef.current = journal;

    const subscribeOptions: JournalSubscribeOptions = {
      fromSequence: options.fromSequence,
      mapName: options.mapName,
      types: options.types,
    };

    const unsubscribe = journal.subscribe((event) => {
      if (!isMounted.current) return;

      // Add to events with rotation
      setEvents((prev) => {
        const newEvents = [...prev, event];
        if (newEvents.length > maxEvents) {
          return newEvents.slice(-maxEvents);
        }
        return newEvents;
      });

      setLastEvent(event);

      // Call event callback
      optionsRef.current.onEvent?.(event);
    }, subscribeOptions);

    setIsSubscribed(true);

    return () => {
      isMounted.current = false;
      setIsSubscribed(false);
      unsubscribe();
    };
  }, [client, filterKey, maxEvents]);

  return useMemo(
    () => ({
      events,
      lastEvent,
      clearEvents,
      readFrom,
      getLatestSequence,
      isSubscribed,
    }),
    [events, lastEvent, clearEvents, readFrom, getLatestSequence, isSubscribed]
  );
}
