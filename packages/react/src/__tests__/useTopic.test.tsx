import { renderHook, act } from '@testing-library/react';
import { useTopic } from '../hooks/useTopic';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock TopicHandle
const createMockTopicHandle = () => {
  const listeners = new Set<(data: any, context: any) => void>();
  return {
    id: 'test-topic',
    subscribe: jest.fn((callback: (data: any, context: any) => void) => {
      listeners.add(callback);
      return () => {
        listeners.delete(callback);
      };
    }),
    publish: jest.fn(),
    // Helper to trigger messages
    _triggerMessage: (data: any, context: any = { timestamp: Date.now() }) => {
      listeners.forEach((cb) => cb(data, context));
    },
    _getListenerCount: () => listeners.size,
  };
};

let mockTopicHandle: ReturnType<typeof createMockTopicHandle>;

const mockTopic = jest.fn(() => mockTopicHandle);
const mockClient = {
  topic: mockTopic,
} as unknown as TopGunClient;

describe('useTopic', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockTopicHandle = createMockTopicHandle();
    mockTopic.mockReturnValue(mockTopicHandle);
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should return a topic handle', () => {
    const { result } = renderHook(() => useTopic('test-topic'), { wrapper });

    expect(result.current).toBe(mockTopicHandle);
    expect(mockTopic).toHaveBeenCalledWith('test-topic');
  });

  it('should subscribe to topic when callback is provided', () => {
    const callback = jest.fn();
    renderHook(() => useTopic('test-topic', callback), { wrapper });

    expect(mockTopicHandle.subscribe).toHaveBeenCalled();
  });

  it('should not subscribe when no callback is provided', () => {
    renderHook(() => useTopic('test-topic'), { wrapper });

    expect(mockTopicHandle.subscribe).not.toHaveBeenCalled();
  });

  it('should call callback when message is received', () => {
    const callback = jest.fn();
    renderHook(() => useTopic('test-topic', callback), { wrapper });

    const testData = { message: 'hello' };
    const testContext = { timestamp: 12345, publisherId: 'node-1' };

    act(() => {
      mockTopicHandle._triggerMessage(testData, testContext);
    });

    expect(callback).toHaveBeenCalledWith(testData, testContext);
  });

  it('should handle multiple messages', () => {
    const callback = jest.fn();
    renderHook(() => useTopic('test-topic', callback), { wrapper });

    act(() => {
      mockTopicHandle._triggerMessage({ msg: 1 }, { timestamp: 1 });
      mockTopicHandle._triggerMessage({ msg: 2 }, { timestamp: 2 });
      mockTopicHandle._triggerMessage({ msg: 3 }, { timestamp: 3 });
    });

    expect(callback).toHaveBeenCalledTimes(3);
    expect(callback).toHaveBeenNthCalledWith(1, { msg: 1 }, { timestamp: 1 });
    expect(callback).toHaveBeenNthCalledWith(2, { msg: 2 }, { timestamp: 2 });
    expect(callback).toHaveBeenNthCalledWith(3, { msg: 3 }, { timestamp: 3 });
  });

  it('should unsubscribe on unmount', () => {
    const callback = jest.fn();
    const { unmount } = renderHook(() => useTopic('test-topic', callback), { wrapper });

    expect(mockTopicHandle._getListenerCount()).toBe(1);

    unmount();

    expect(mockTopicHandle._getListenerCount()).toBe(0);
  });

  it('should allow publishing messages via returned topic handle', () => {
    const { result } = renderHook(() => useTopic('test-topic'), { wrapper });

    const testData = { action: 'test' };
    result.current.publish(testData);

    expect(mockTopicHandle.publish).toHaveBeenCalledWith(testData);
  });

  it('should not call callback after unmount', () => {
    const callback = jest.fn();
    const { unmount } = renderHook(() => useTopic('test-topic', callback), { wrapper });

    // Trigger a message before unmount
    act(() => {
      mockTopicHandle._triggerMessage({ before: true }, { timestamp: 1 });
    });
    expect(callback).toHaveBeenCalledTimes(1);

    unmount();
    callback.mockClear();

    // The listener should be removed, so no callback
    act(() => {
      mockTopicHandle._triggerMessage({ after: true }, { timestamp: 2 });
    });
    expect(callback).not.toHaveBeenCalled();
  });

  it('should update callback ref when callback changes', () => {
    const callback1 = jest.fn();
    const callback2 = jest.fn();

    const { rerender } = renderHook(({ callback }) => useTopic('test-topic', callback), {
      wrapper,
      initialProps: { callback: callback1 },
    });

    act(() => {
      mockTopicHandle._triggerMessage({ msg: 'first' }, { timestamp: 1 });
    });
    expect(callback1).toHaveBeenCalledWith({ msg: 'first' }, { timestamp: 1 });
    expect(callback2).not.toHaveBeenCalled();

    rerender({ callback: callback2 });

    act(() => {
      mockTopicHandle._triggerMessage({ msg: 'second' }, { timestamp: 2 });
    });
    expect(callback2).toHaveBeenCalledWith({ msg: 'second' }, { timestamp: 2 });
  });

  // TODO-516 regression: previously the subscription effect depended on
  // [topic, callback], so a fresh inline callback every render tore down and
  // re-created the subscription on EVERY render — the callbackRef indirection
  // was dead code. The migration keys the subscription only on topic identity
  // (callback presence, not identity), so a changing inline callback must NOT
  // re-subscribe, and the latest callback must still receive messages.
  it('does NOT re-subscribe when only the inline callback identity changes (TODO-516)', () => {
    // Render with a NEW inline callback every render — the common
    // useTopic('chat', d => …) usage. We capture the latest closure so we can
    // assert the most recent one still fires.
    const received: any[] = [];
    const { rerender } = renderHook(
      // Intentionally a fresh inline callback per render to exercise the churn fix.
      ({ tag }: { tag: number }) =>
        useTopic('test-topic', (data: any) => {
          received.push({ tag, data });
        }),
      { wrapper, initialProps: { tag: 0 } },
    );

    // Exactly one subscribe across the initial render(s).
    expect(mockTopicHandle.subscribe).toHaveBeenCalledTimes(1);
    expect(mockTopicHandle._getListenerCount()).toBe(1);

    // Force several rerenders, each with a brand-new inline callback identity.
    rerender({ tag: 1 });
    rerender({ tag: 2 });
    rerender({ tag: 3 });

    // STILL exactly one subscribe — no churn.
    expect(mockTopicHandle.subscribe).toHaveBeenCalledTimes(1);
    expect(mockTopicHandle._getListenerCount()).toBe(1);

    // The LATEST callback (tag 3) must receive the message.
    act(() => {
      mockTopicHandle._triggerMessage({ msg: 'hello' }, { timestamp: 9 });
    });
    expect(received).toHaveLength(1);
    expect(received[0]).toEqual({ tag: 3, data: { msg: 'hello' } });
  });

  it('StrictMode double-invoke leaves no leaked topic listener', () => {
    const callback = jest.fn();

    const StrictWrapper = ({ children }: { children: React.ReactNode }) => (
      <React.StrictMode>
        <TopGunProvider client={mockClient}>{children}</TopGunProvider>
      </React.StrictMode>
    );

    const { unmount } = renderHook(() => useTopic('test-topic', callback), {
      wrapper: StrictWrapper,
    });

    // After mount (even with StrictMode's mount→unmount→mount), exactly one
    // live listener remains.
    expect(mockTopicHandle._getListenerCount()).toBe(1);

    unmount();
    // Net-zero: no leaked listener.
    expect(mockTopicHandle._getListenerCount()).toBe(0);
  });
});
