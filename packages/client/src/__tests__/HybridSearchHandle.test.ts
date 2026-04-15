/**
 * HybridSearchHandle Tests
 *
 * Unit tests for HybridSearchHandle — the live tri-hybrid search subscription handle.
 * Covers AC #1–#9 from SPEC-211.
 *
 * Test harness mirrors the SearchHandle section of Search.test.ts:
 * a real SyncEngine with a mocked sendMessage spy, driven by handleServerMessage.
 */

import { SyncEngine } from '../SyncEngine';
import { HybridSearchHandle } from '../HybridSearchHandle';
import { SyncState } from '../SyncState';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { vectorToBytes } from '@topgunbuild/core';

// Mock storage adapter
const createMockStorage = () => ({
  initialize: jest.fn().mockResolvedValue(undefined),
  get: jest.fn().mockResolvedValue(null),
  put: jest.fn().mockResolvedValue(undefined),
  remove: jest.fn().mockResolvedValue(undefined),
  getAllKeys: jest.fn().mockResolvedValue([]),
  getMeta: jest.fn().mockResolvedValue(null),
  setMeta: jest.fn().mockResolvedValue(undefined),
  clear: jest.fn().mockResolvedValue(undefined),
  getPendingOps: jest.fn().mockResolvedValue([]),
  savePendingOps: jest.fn().mockResolvedValue(undefined),
  clearPendingOps: jest.fn().mockResolvedValue(undefined),
});

// Minimal WebSocket mock (same shape as Search.test.ts)
(globalThis as any).WebSocket = class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 1;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: ((error: Error) => void) | null = null;
  send = jest.fn();
  close = jest.fn();
};

describe('HybridSearchHandle', () => {
  let syncEngine: SyncEngine;
  let mockSendMessage: jest.SpyInstance;

  beforeEach(() => {
    jest.useFakeTimers();

    syncEngine = new SyncEngine({
      nodeId: 'test-node',
      connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
      storageAdapter: createMockStorage() as any,
    });

    mockSendMessage = jest.spyOn(syncEngine as any, 'sendMessage').mockReturnValue(true);
    (syncEngine as any).stateMachine.state = SyncState.CONNECTED;
  });

  afterEach(() => {
    jest.useRealTimers();
    syncEngine.close();
  });

  // ============================================
  // AC #1 — Constructor sends HYBRID_SEARCH_SUB with valid UUID and correct fields
  // ============================================

  it('AC #1: sends HYBRID_SEARCH_SUB immediately with UUID subscriptionId and correct payload', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query', {
      methods: ['fullText', 'semantic'],
      k: 5,
    });

    expect(mockSendMessage).toHaveBeenCalledTimes(1);
    const msg = mockSendMessage.mock.calls[0][0];
    expect(msg.type).toBe('HYBRID_SEARCH_SUB');
    expect(msg.payload.subscriptionId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
    );
    expect(msg.payload.mapName).toBe('docs');
    expect(msg.payload.queryText).toBe('query');
    expect(msg.payload.methods).toEqual(['fullText', 'semantic']);
    expect(msg.payload.k).toBe(5);

    handle.dispose();
  });

  // ============================================
  // AC #2 — queryVector serialized to little-endian Uint8Array via vectorToBytes
  // ============================================

  it('AC #2: serializes queryVector to little-endian Uint8Array via vectorToBytes', () => {
    const vec = new Float32Array([0.1, 0.2]);
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query', {
      queryVector: vec,
    });

    const msg = mockSendMessage.mock.calls[0][0];
    const expectedBytes = vectorToBytes(vec);

    expect(msg.payload.queryVector).toBeInstanceOf(Uint8Array);
    expect(msg.payload.queryVector).toEqual(expectedBytes);

    handle.dispose();
  });

  // ============================================
  // AC #3 — HYBRID_SEARCH_RESP matching subscriptionId populates results, sorted desc
  // ============================================

  it('AC #3: populates results and notifies subscribers when HYBRID_SEARCH_RESP matches subscriptionId', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear(); // ignore initial empty call

    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_RESP',
      payload: {
        requestId: subscriptionId,
        results: [
          { key: 'doc2', score: 1.0, methodScores: { fullText: 1.0 }, value: { title: 'Low' } },
          { key: 'doc1', score: 2.5, methodScores: { fullText: 2.5 }, value: { title: 'High' } },
        ],
        searchTimeMs: 5,
      },
    });

    const results = handle.getResults();
    expect(results).toHaveLength(2);
    expect(results[0].key).toBe('doc1');   // highest score first
    expect(results[0].score).toBe(2.5);
    expect(results[1].key).toBe('doc2');
    expect(callback).toHaveBeenCalledTimes(1);
    expect(callback).toHaveBeenCalledWith(results);

    handle.dispose();
  });

  // ============================================
  // AC #4 — HYBRID_SEARCH_RESP with non-matching requestId is ignored
  // ============================================

  it('AC #4: ignores HYBRID_SEARCH_RESP with non-matching requestId', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear();

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_RESP',
      payload: {
        requestId: 'wrong-id',
        results: [
          { key: 'doc1', score: 1.0, methodScores: {}, value: { title: 'Test' } },
        ],
        searchTimeMs: 1,
      },
    });

    expect(handle.getResults()).toHaveLength(0);
    expect(callback).not.toHaveBeenCalled();

    handle.dispose();
  });

  // ============================================
  // AC #5 — HYBRID_SEARCH_UPDATE: ENTER/UPDATE/LEAVE deltas applied correctly
  // ============================================

  it('AC #5a: ENTER adds key to results and notifies subscribers', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear();

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 1.5,
        methodScores: { fullText: 1.5 },
        value: { title: 'Hello' },
        changeType: 'ENTER',
      },
    });

    expect(handle.getResults()).toHaveLength(1);
    expect(handle.getResults()[0].key).toBe('doc1');
    expect(callback).toHaveBeenCalledTimes(1);

    handle.dispose();
  });

  it('AC #5b: UPDATE mutates score, methodScores, and value in place', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    // Add via ENTER first
    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 1.0,
        methodScores: { fullText: 1.0 },
        value: { title: 'Original' },
        changeType: 'ENTER',
      },
    });

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear();

    // UPDATE the same key
    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 3.0,
        methodScores: { fullText: 2.0, semantic: 1.0 },
        value: { title: 'Updated' },
        changeType: 'UPDATE',
      },
    });

    const results = handle.getResults();
    expect(results).toHaveLength(1);
    expect(results[0].score).toBe(3.0);
    expect(results[0].methodScores).toEqual({ fullText: 2.0, semantic: 1.0 });
    expect((results[0].value as any).title).toBe('Updated');
    expect(callback).toHaveBeenCalledTimes(1);

    handle.dispose();
  });

  it('AC #5c: LEAVE removes key from results', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 1.0,
        methodScores: {},
        changeType: 'ENTER',
      },
    });

    expect(handle.getResults()).toHaveLength(1);

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear();

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 0,
        methodScores: {},
        changeType: 'LEAVE',
      },
    });

    expect(handle.getResults()).toHaveLength(0);
    expect(callback).toHaveBeenCalledTimes(1);
    expect(callback).toHaveBeenCalledWith([]);

    handle.dispose();
  });

  // ============================================
  // AC #6 — HYBRID_SEARCH_UPDATE with non-matching subscriptionId is ignored
  // ============================================

  it('AC #6: ignores HYBRID_SEARCH_UPDATE with non-matching subscriptionId', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear();

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId: 'wrong-id',
        key: 'doc1',
        score: 1.0,
        methodScores: {},
        changeType: 'ENTER',
      },
    });

    expect(handle.getResults()).toHaveLength(0);
    expect(callback).not.toHaveBeenCalled();

    handle.dispose();
  });

  // ============================================
  // AC #7 — setQuery sends UNSUB/SUB, clears results, notifies with empty array, new UUID
  // ============================================

  it('AC #7: setQuery sends UNSUB for old subscriptionId then SUB with new UUID subscriptionId', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'old query');
    const oldSubId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    // Populate some results so we can verify they're cleared
    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId: oldSubId,
        key: 'doc1',
        score: 1.0,
        methodScores: {},
        changeType: 'ENTER',
      },
    });
    expect(handle.getResults()).toHaveLength(1);

    const callback = jest.fn();
    handle.subscribe(callback);
    callback.mockClear();

    mockSendMessage.mockClear();
    handle.setQuery('new query');

    // First call: UNSUB for old ID
    expect(mockSendMessage.mock.calls[0][0]).toMatchObject({
      type: 'HYBRID_SEARCH_UNSUB',
      payload: { subscriptionId: oldSubId },
    });

    // Second call: SUB with NEW ID
    const subCall = mockSendMessage.mock.calls[1][0];
    expect(subCall.type).toBe('HYBRID_SEARCH_SUB');
    expect(subCall.payload.queryText).toBe('new query');
    expect(subCall.payload.subscriptionId).not.toBe(oldSubId);
    expect(subCall.payload.subscriptionId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
    );

    // Results cleared, subscribers notified with empty array
    expect(handle.getResults()).toHaveLength(0);
    expect(callback).toHaveBeenCalledWith([]);

    handle.dispose();
  });

  // ============================================
  // AC #8 — dispose sends UNSUB, detaches listener, clears state; double-dispose is no-op
  // ============================================

  it('AC #8: dispose sends HYBRID_SEARCH_UNSUB, detaches listener, clears state; calling dispose twice is a no-op', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    // Add a result so clear() is observable
    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 1.0,
        methodScores: {},
        changeType: 'ENTER',
      },
    });

    mockSendMessage.mockClear();
    handle.dispose();

    // UNSUB was sent
    expect(mockSendMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'HYBRID_SEARCH_UNSUB',
        payload: { subscriptionId },
      })
    );

    expect(handle.isDisposed()).toBe(true);
    expect(handle.size).toBe(0);

    // Second dispose: no additional UNSUB, no throw
    mockSendMessage.mockClear();
    handle.dispose();
    expect(mockSendMessage).not.toHaveBeenCalled();
  });

  // ============================================
  // AC #9 — throwing listener does not prevent other listeners from being called
  // ============================================

  it('AC #9: a listener that throws does not prevent other listeners from firing', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    let shouldThrow = false;
    const throwingListener = jest.fn().mockImplementation(() => {
      if (shouldThrow) {
        throw new Error('listener boom');
      }
    });
    const safeListener = jest.fn();

    handle.subscribe(throwingListener);
    handle.subscribe(safeListener);
    // Clear initial invocations from subscribe
    throwingListener.mockClear();
    safeListener.mockClear();

    // Arm the throw before the next notification
    shouldThrow = true;

    // This should not throw despite the first listener throwing
    expect(() => {
      (syncEngine as any).handleServerMessage({
        type: 'HYBRID_SEARCH_UPDATE',
        payload: {
          subscriptionId,
          key: 'doc1',
          score: 1.0,
          methodScores: {},
          changeType: 'ENTER',
        },
      });
    }).not.toThrow();

    expect(throwingListener).toHaveBeenCalledTimes(1);
    expect(safeListener).toHaveBeenCalledTimes(1);

    handle.dispose();
  });

  // ============================================
  // Additional: subscribe throws on disposed handle
  // ============================================

  it('throws when subscribing to a disposed handle', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    handle.dispose();
    expect(() => handle.subscribe(() => {})).toThrow('HybridSearchHandle has been disposed');
  });

  it('throws when calling setQuery on a disposed handle', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    handle.dispose();
    expect(() => handle.setQuery('new')).toThrow('HybridSearchHandle has been disposed');
  });

  it('throws when calling setOptions on a disposed handle', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    handle.dispose();
    expect(() => handle.setOptions({ k: 5 })).toThrow('HybridSearchHandle has been disposed');
  });

  // ============================================
  // Additional: setQuery with same value is a no-op
  // ============================================

  it('setQuery with same queryText does not resend messages', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'same');
    mockSendMessage.mockClear();
    handle.setQuery('same');
    expect(mockSendMessage).not.toHaveBeenCalled();
    handle.dispose();
  });

  // ============================================
  // Additional: subscribe returns working unsubscribe function
  // ============================================

  it('unsubscribe function removes listener from further notifications', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const subscriptionId = mockSendMessage.mock.calls[0][0].payload.subscriptionId;

    const callback = jest.fn();
    const unsubscribe = handle.subscribe(callback);
    callback.mockClear();

    unsubscribe();

    (syncEngine as any).handleServerMessage({
      type: 'HYBRID_SEARCH_UPDATE',
      payload: {
        subscriptionId,
        key: 'doc1',
        score: 1.0,
        methodScores: {},
        changeType: 'ENTER',
      },
    });

    expect(callback).not.toHaveBeenCalled();

    handle.dispose();
  });

  // ============================================
  // Additional: defaults applied when options omitted
  // ============================================

  it('applies default methods=[fullText] and k=10 when options omitted', () => {
    const handle = new HybridSearchHandle(syncEngine, 'docs', 'query');
    const msg = mockSendMessage.mock.calls[0][0];
    expect(msg.payload.methods).toEqual(['fullText']);
    expect(msg.payload.k).toBe(10);
    handle.dispose();
  });
});
