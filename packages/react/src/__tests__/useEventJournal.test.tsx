import { renderHook, act } from '@testing-library/react';
import { useEventJournal } from '../hooks/useEventJournal';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import type { JournalEvent } from '@topgunbuild/core';
import React from 'react';

// Mock EventJournalReader
const createMockJournalReader = () => {
  const listeners = new Map<string, (event: JournalEvent) => void>();
  let subscriptionCounter = 0;

  return {
    subscribe: jest.fn((callback: (event: JournalEvent) => void, options?: any) => {
      const id = `sub_${subscriptionCounter++}`;
      listeners.set(id, callback);
      return () => {
        listeners.delete(id);
      };
    }),
    readFrom: jest.fn().mockResolvedValue([]),
    getLatestSequence: jest.fn().mockResolvedValue(0n),
    // Test helpers
    _triggerEvent: (event: JournalEvent) => {
      listeners.forEach((cb) => cb(event));
    },
    _getListenerCount: () => listeners.size,
    _reset: () => {
      listeners.clear();
      subscriptionCounter = 0;
    },
  };
};

const createMockEvent = (overrides: Partial<JournalEvent> = {}): JournalEvent => ({
  sequence: 0n,
  type: 'PUT',
  mapName: 'testMap',
  key: 'testKey',
  value: { data: 'test' },
  timestamp: { millis: Date.now(), counter: 0, nodeId: 'node1' },
  nodeId: 'node1',
  ...overrides,
});

describe('useEventJournal', () => {
  let mockJournalReader: ReturnType<typeof createMockJournalReader>;
  let mockClient: TopGunClient;

  beforeEach(() => {
    jest.clearAllMocks();
    mockJournalReader = createMockJournalReader();
    mockClient = {
      getEventJournal: jest.fn().mockReturnValue(mockJournalReader),
    } as unknown as TopGunClient;
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  describe('initialization', () => {
    it('should initialize with empty events and isSubscribed true', () => {
      const { result } = renderHook(() => useEventJournal(), { wrapper });

      expect(result.current.events).toEqual([]);
      expect(result.current.lastEvent).toBeNull();
      expect(result.current.isSubscribed).toBe(true);
    });

    it('should call getEventJournal on the client', () => {
      renderHook(() => useEventJournal(), { wrapper });

      expect(mockClient.getEventJournal).toHaveBeenCalled();
    });

    it('should subscribe to journal events', () => {
      renderHook(() => useEventJournal(), { wrapper });

      expect(mockJournalReader.subscribe).toHaveBeenCalled();
    });
  });

  describe('receiving events', () => {
    it('should add events to the events array', () => {
      const { result } = renderHook(() => useEventJournal(), { wrapper });

      const event1 = createMockEvent({ sequence: 1n, key: 'key1' });
      const event2 = createMockEvent({ sequence: 2n, key: 'key2' });

      act(() => {
        mockJournalReader._triggerEvent(event1);
      });

      expect(result.current.events).toHaveLength(1);
      expect(result.current.events[0].key).toBe('key1');

      act(() => {
        mockJournalReader._triggerEvent(event2);
      });

      expect(result.current.events).toHaveLength(2);
      expect(result.current.events[1].key).toBe('key2');
    });

    it('should update lastEvent when new event is received', () => {
      const { result } = renderHook(() => useEventJournal(), { wrapper });

      const event = createMockEvent({ sequence: 1n, key: 'newKey' });

      act(() => {
        mockJournalReader._triggerEvent(event);
      });

      expect(result.current.lastEvent).not.toBeNull();
      expect(result.current.lastEvent?.key).toBe('newKey');
    });

    it('should call onEvent callback when event is received', () => {
      const onEvent = jest.fn();
      renderHook(() => useEventJournal({ onEvent }), { wrapper });

      const event = createMockEvent({ sequence: 1n });

      act(() => {
        mockJournalReader._triggerEvent(event);
      });

      expect(onEvent).toHaveBeenCalledWith(event);
    });
  });

  describe('maxEvents option', () => {
    it('should limit events to maxEvents (default 100)', () => {
      const { result } = renderHook(() => useEventJournal({ maxEvents: 5 }), { wrapper });

      // Add 7 events
      act(() => {
        for (let i = 0; i < 7; i++) {
          mockJournalReader._triggerEvent(createMockEvent({ sequence: BigInt(i), key: `key${i}` }));
        }
      });

      expect(result.current.events).toHaveLength(5);
      // Should keep the last 5 events
      expect(result.current.events[0].key).toBe('key2');
      expect(result.current.events[4].key).toBe('key6');
    });

    it('should use default maxEvents of 100', () => {
      const { result } = renderHook(() => useEventJournal(), { wrapper });

      // Add 105 events
      act(() => {
        for (let i = 0; i < 105; i++) {
          mockJournalReader._triggerEvent(createMockEvent({ sequence: BigInt(i), key: `key${i}` }));
        }
      });

      expect(result.current.events).toHaveLength(100);
      // Should keep last 100 events (5-104)
      expect(result.current.events[0].key).toBe('key5');
      expect(result.current.events[99].key).toBe('key104');
    });
  });

  describe('filtering options', () => {
    it('should pass mapName filter to subscribe', () => {
      renderHook(() => useEventJournal({ mapName: 'users' }), { wrapper });

      expect(mockJournalReader.subscribe).toHaveBeenCalledWith(
        expect.any(Function),
        expect.objectContaining({ mapName: 'users' })
      );
    });

    it('should pass types filter to subscribe', () => {
      renderHook(() => useEventJournal({ types: ['PUT', 'DELETE'] }), { wrapper });

      expect(mockJournalReader.subscribe).toHaveBeenCalledWith(
        expect.any(Function),
        expect.objectContaining({ types: ['PUT', 'DELETE'] })
      );
    });

    it('should pass fromSequence filter to subscribe', () => {
      renderHook(() => useEventJournal({ fromSequence: 100n }), { wrapper });

      expect(mockJournalReader.subscribe).toHaveBeenCalledWith(
        expect.any(Function),
        expect.objectContaining({ fromSequence: 100n })
      );
    });
  });

  describe('paused option', () => {
    it('should not subscribe when paused is true', () => {
      const { result } = renderHook(() => useEventJournal({ paused: true }), { wrapper });

      expect(mockJournalReader.subscribe).not.toHaveBeenCalled();
      expect(result.current.isSubscribed).toBe(false);
    });

    it('should subscribe when paused changes from true to false', () => {
      const { result, rerender } = renderHook(
        ({ paused }) => useEventJournal({ paused }),
        { wrapper, initialProps: { paused: true } }
      );

      expect(result.current.isSubscribed).toBe(false);
      expect(mockJournalReader.subscribe).not.toHaveBeenCalled();

      rerender({ paused: false });

      expect(result.current.isSubscribed).toBe(true);
      expect(mockJournalReader.subscribe).toHaveBeenCalled();
    });

    it('should unsubscribe when paused changes from false to true', () => {
      const { result, rerender } = renderHook(
        ({ paused }) => useEventJournal({ paused }),
        { wrapper, initialProps: { paused: false } }
      );

      expect(result.current.isSubscribed).toBe(true);
      expect(mockJournalReader._getListenerCount()).toBe(1);

      rerender({ paused: true });

      expect(result.current.isSubscribed).toBe(false);
      expect(mockJournalReader._getListenerCount()).toBe(0);
    });
  });

  describe('clearEvents', () => {
    it('should clear all events', () => {
      const { result } = renderHook(() => useEventJournal(), { wrapper });

      // Add some events
      act(() => {
        mockJournalReader._triggerEvent(createMockEvent({ sequence: 1n }));
        mockJournalReader._triggerEvent(createMockEvent({ sequence: 2n }));
      });

      expect(result.current.events).toHaveLength(2);
      expect(result.current.lastEvent).not.toBeNull();

      act(() => {
        result.current.clearEvents();
      });

      expect(result.current.events).toHaveLength(0);
      expect(result.current.lastEvent).toBeNull();
    });
  });

  describe('readFrom', () => {
    it('should call readFrom on the journal reader', async () => {
      const mockEvents = [
        createMockEvent({ sequence: 1n }),
        createMockEvent({ sequence: 2n }),
      ];
      mockJournalReader.readFrom.mockResolvedValue(mockEvents);

      const { result } = renderHook(() => useEventJournal(), { wrapper });

      let events: JournalEvent[] = [];
      await act(async () => {
        events = await result.current.readFrom(0n, 50);
      });

      expect(mockJournalReader.readFrom).toHaveBeenCalledWith(0n, 50);
      expect(events).toEqual(mockEvents);
    });

    it('should use default limit if not provided', async () => {
      mockJournalReader.readFrom.mockResolvedValue([]);

      const { result } = renderHook(() => useEventJournal(), { wrapper });

      await act(async () => {
        await result.current.readFrom(10n);
      });

      expect(mockJournalReader.readFrom).toHaveBeenCalledWith(10n, undefined);
    });
  });

  describe('getLatestSequence', () => {
    it('should call getLatestSequence on the journal reader', async () => {
      mockJournalReader.getLatestSequence.mockResolvedValue(42n);

      const { result } = renderHook(() => useEventJournal(), { wrapper });

      let sequence: bigint = 0n;
      await act(async () => {
        sequence = await result.current.getLatestSequence();
      });

      expect(mockJournalReader.getLatestSequence).toHaveBeenCalled();
      expect(sequence).toBe(42n);
    });
  });

  describe('cleanup', () => {
    it('should unsubscribe on unmount', () => {
      const { unmount } = renderHook(() => useEventJournal(), { wrapper });

      expect(mockJournalReader._getListenerCount()).toBe(1);

      unmount();

      expect(mockJournalReader._getListenerCount()).toBe(0);
    });

    it('should not call event handlers after unmount', () => {
      const onEvent = jest.fn();
      const { unmount } = renderHook(() => useEventJournal({ onEvent }), { wrapper });

      // Trigger event before unmount
      act(() => {
        mockJournalReader._triggerEvent(createMockEvent({ sequence: 1n }));
      });
      expect(onEvent).toHaveBeenCalledTimes(1);

      unmount();
      onEvent.mockClear();

      // Trigger event after unmount (listener should be removed)
      act(() => {
        mockJournalReader._triggerEvent(createMockEvent({ sequence: 2n }));
      });
      expect(onEvent).not.toHaveBeenCalled();
    });
  });

  describe('re-subscription on option changes', () => {
    it('should re-subscribe when mapName changes', () => {
      const { rerender } = renderHook(
        ({ mapName }) => useEventJournal({ mapName }),
        { wrapper, initialProps: { mapName: 'users' } }
      );

      expect(mockJournalReader.subscribe).toHaveBeenCalledTimes(1);
      expect(mockJournalReader.subscribe).toHaveBeenLastCalledWith(
        expect.any(Function),
        expect.objectContaining({ mapName: 'users' })
      );

      rerender({ mapName: 'orders' });

      expect(mockJournalReader.subscribe).toHaveBeenCalledTimes(2);
      expect(mockJournalReader.subscribe).toHaveBeenLastCalledWith(
        expect.any(Function),
        expect.objectContaining({ mapName: 'orders' })
      );
    });

    it('should re-subscribe when types change', () => {
      const { rerender } = renderHook(
        ({ types }) => useEventJournal({ types }),
        { wrapper, initialProps: { types: ['PUT'] as JournalEvent['type'][] } }
      );

      expect(mockJournalReader.subscribe).toHaveBeenCalledTimes(1);

      rerender({ types: ['PUT', 'DELETE'] as JournalEvent['type'][] });

      expect(mockJournalReader.subscribe).toHaveBeenCalledTimes(2);
    });
  });

  describe('callback reference stability', () => {
    it('should use latest onEvent callback without re-subscribing', () => {
      const onEvent1 = jest.fn();
      const onEvent2 = jest.fn();

      const { rerender } = renderHook(
        ({ onEvent }) => useEventJournal({ onEvent }),
        { wrapper, initialProps: { onEvent: onEvent1 } }
      );

      // First event with first callback
      act(() => {
        mockJournalReader._triggerEvent(createMockEvent({ sequence: 1n, key: 'first' }));
      });
      expect(onEvent1).toHaveBeenCalledTimes(1);
      expect(onEvent2).not.toHaveBeenCalled();

      // Change callback - should not re-subscribe
      const subscribeCountBefore = mockJournalReader.subscribe.mock.calls.length;
      rerender({ onEvent: onEvent2 });
      expect(mockJournalReader.subscribe).toHaveBeenCalledTimes(subscribeCountBefore);

      // Second event should use new callback
      act(() => {
        mockJournalReader._triggerEvent(createMockEvent({ sequence: 2n, key: 'second' }));
      });
      expect(onEvent1).toHaveBeenCalledTimes(1); // Still 1
      expect(onEvent2).toHaveBeenCalledTimes(1); // Now called
    });
  });

  describe('multiple event types', () => {
    it('should handle PUT, UPDATE, and DELETE events', () => {
      const { result } = renderHook(() => useEventJournal(), { wrapper });

      act(() => {
        mockJournalReader._triggerEvent(createMockEvent({
          sequence: 1n,
          type: 'PUT',
          key: 'user1',
          value: { name: 'Alice' },
        }));
        mockJournalReader._triggerEvent(createMockEvent({
          sequence: 2n,
          type: 'UPDATE',
          key: 'user1',
          value: { name: 'Alice Updated' },
          previousValue: { name: 'Alice' },
        }));
        mockJournalReader._triggerEvent(createMockEvent({
          sequence: 3n,
          type: 'DELETE',
          key: 'user1',
          value: undefined,
          previousValue: { name: 'Alice Updated' },
        }));
      });

      expect(result.current.events).toHaveLength(3);
      expect(result.current.events[0].type).toBe('PUT');
      expect(result.current.events[1].type).toBe('UPDATE');
      expect(result.current.events[2].type).toBe('DELETE');
    });
  });
});
