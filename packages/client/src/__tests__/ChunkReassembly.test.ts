/**
 * Reassembly of `CHUNK` sequences in WebSocketManager (TG-WS-002).
 *
 * A server sends a message longer than its chunk length to an opted-in client
 * as consecutive `CHUNK` messages whose `data` concatenates to the original
 * bytes. The client must route only the reassembled message, never a `CHUNK`,
 * must route a whole message that arrives inside an open sequence at once
 * (the server writes small messages between the `CHUNK`s of a large answer,
 * TG-WS-004), and must treat every sequence it cannot reassemble or decode as
 * a protocol violation that forces a reconnect, so the message is recovered by
 * resync.
 */

import { serialize } from '@topgunbuild/core';
import { ChunkReassembler, MAX_REASSEMBLY_BYTES, WebSocketManager } from '../sync/WebSocketManager';
import { SyncStateMachine } from '../SyncStateMachine';
import type { IConnectionProvider } from '../types';

type Handler = (...args: unknown[]) => void;

class FakeProvider {
  private handlers = new Map<string, Set<Handler>>();
  forceReconnect = jest.fn();

  on(event: string, handler: Handler): void {
    if (!this.handlers.has(event)) this.handlers.set(event, new Set());
    this.handlers.get(event)!.add(handler);
  }

  off(event: string, handler: Handler): void {
    this.handlers.get(event)?.delete(handler);
  }

  isConnected(): boolean {
    return true;
  }

  emit(event: string, ...args: unknown[]): void {
    for (const handler of this.handlers.get(event) ?? []) handler(...args);
  }

  /** Delivers one server message as the platform WebSocket would: an ArrayBuffer. */
  deliver(message: unknown, nodeId = 'default'): void {
    this.deliverBytes(serialize(message), nodeId);
  }

  deliverBytes(bytes: Uint8Array, nodeId = 'default'): void {
    this.emit('message', nodeId, new Uint8Array(bytes).buffer);
  }
}

function chunk(
  last: boolean,
  data: Uint8Array,
): { type: 'CHUNK'; last: boolean; data: Uint8Array } {
  return { type: 'CHUNK', last, data };
}

/** Splits `bytes` at the given offsets into a CHUNK sequence. */
function chunksOf(bytes: Uint8Array, offsets: number[]): ReturnType<typeof chunk>[] {
  const bounds = [0, ...offsets, bytes.length];
  const out: ReturnType<typeof chunk>[] = [];
  for (let i = 0; i + 1 < bounds.length; i++) {
    out.push(chunk(i + 2 === bounds.length, bytes.slice(bounds[i], bounds[i + 1])));
  }
  return out;
}

const original = {
  type: 'QUERY_UPDATE',
  payload: {
    queryId: 'q-1',
    key: 'row-7',
    value: { title: 'a title long enough to be split', n: 42, tags: ['x', 'y', 'z'] },
    changeType: 'UPDATE',
  },
};

const whole = { type: 'QUERY_UPDATE', payload: { queryId: 'q-2', key: 'k', value: 1 } };

function setup(): {
  provider: FakeProvider;
  manager: WebSocketManager;
  onMessage: jest.Mock;
} {
  const provider = new FakeProvider();
  const onMessage = jest.fn();
  const manager = new WebSocketManager({
    connectionProvider: provider as unknown as IConnectionProvider,
    stateMachine: new SyncStateMachine(),
    heartbeatConfig: { intervalMs: 5000, timeoutMs: 15000, enabled: false },
    onMessage,
  });
  return { provider, manager, onMessage };
}

describe('CHUNK reassembly (TG-WS-002)', () => {
  describe('T5: a sequence is routed as one message', () => {
    it('routes exactly the reassembled message of a 3-CHUNK sequence, and no CHUNK', () => {
      const { provider, onMessage } = setup();
      const bytes = serialize(original);
      const sequence = chunksOf(bytes, [7, 40]);
      expect(sequence).toHaveLength(3);

      provider.deliver(sequence[0]);
      provider.deliver(sequence[1]);
      expect(onMessage).not.toHaveBeenCalled();
      provider.deliver(sequence[2]);

      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith(original);
      expect(onMessage.mock.calls.some(([m]) => m?.type === 'CHUNK')).toBe(false);
      expect(provider.forceReconnect).not.toHaveBeenCalled();
    });

    it('routes a 1-CHUNK sequence (last at once)', () => {
      const { provider, onMessage } = setup();
      provider.deliver(chunk(true, serialize(original)));

      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith(original);
      expect(provider.forceReconnect).not.toHaveBeenCalled();
    });
  });

  describe('T6: buffers are cleared by each connection event on its own', () => {
    it('T6a: disconnected, with no connected after it, drops the partial sequence', () => {
      const { provider, onMessage } = setup();
      provider.deliver(chunk(false, serialize(original).slice(0, 10)));

      provider.emit('disconnected', 'default');
      provider.deliver(whole);

      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenLastCalledWith(whole);
      expect(provider.forceReconnect).not.toHaveBeenCalled();

      for (const c of chunksOf(serialize(original), [5])) provider.deliver(c);
      expect(onMessage).toHaveBeenCalledTimes(2);
      expect(onMessage).toHaveBeenLastCalledWith(original);
      expect(provider.forceReconnect).not.toHaveBeenCalled();
    });

    it('T6b: connected, with no disconnected before it, drops the partial sequence', () => {
      const { provider, onMessage } = setup();
      provider.deliver(chunk(false, serialize(original).slice(0, 10)));

      provider.emit('connected', 'default');
      provider.deliver(whole);

      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenLastCalledWith(whole);
      expect(provider.forceReconnect).not.toHaveBeenCalled();
    });
  });

  describe('T7: a whole message inside a sequence is routed; a malformed CHUNK forces a reconnect', () => {
    it("T7': a whole message inside an open sequence is routed at once, and the sequence completes after it", () => {
      const { provider, onMessage } = setup();
      const sequence = chunksOf(serialize(original), [10, 30]);
      expect(sequence).toHaveLength(3);

      provider.deliver(sequence[0]);
      provider.deliver(whole);
      // Routed when it arrives, not after the sequence ends.
      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith(whole);

      provider.deliver(sequence[1]);
      expect(onMessage).toHaveBeenCalledTimes(1);
      provider.deliver(sequence[2]);

      // The open sequence survived the whole message and its own message follows it.
      expect(onMessage.mock.calls.map(([m]) => m)).toEqual([whole, original]);
      expect(provider.forceReconnect).not.toHaveBeenCalled();
    });

    it('a CHUNK whose last is not a boolean, inside a sequence', () => {
      const { provider, onMessage } = setup();
      const bytes = serialize(original);
      provider.deliver(chunk(false, bytes.slice(0, 10)));
      provider.deliver({ type: 'CHUNK', last: 'yes', data: bytes.slice(10) });

      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
      expect(onMessage).not.toHaveBeenCalled();

      provider.deliver(chunk(true, bytes));
      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith(original);
      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
    });

    it('a CHUNK whose data is not bytes', () => {
      const { provider, onMessage } = setup();
      provider.deliver({ type: 'CHUNK', last: true, data: 'not bytes' });

      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
      expect(onMessage).not.toHaveBeenCalled();
    });
  });

  describe("T7'': a whole message inside a sequence, next to a violation", () => {
    it('(1) CHUNK(last=false), a whole message, CHUNK(last=true): both delivered, the whole one first', () => {
      const { provider, onMessage } = setup();
      const [first, second] = chunksOf(serialize(original), [12]);

      provider.deliver(first);
      provider.deliver(whole);
      provider.deliver(second);

      expect(onMessage.mock.calls.map(([m]) => m)).toEqual([whole, original]);
      expect(provider.forceReconnect).not.toHaveBeenCalled();
    });

    it.each([
      ['with no data', { type: 'CHUNK', last: true }],
      ['whose last is not a boolean', { type: 'CHUNK', last: 1, data: new Uint8Array([1, 2]) }],
    ])(
      '(2) CHUNK(last=false), then a malformed CHUNK %s: one reconnect and the sequence discarded',
      (_label, malformed) => {
        const { provider, onMessage } = setup();
        const bytes = serialize(original);
        provider.deliver(chunk(false, bytes.slice(0, 10)));
        provider.deliver(malformed);

        expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
        expect(onMessage).not.toHaveBeenCalled();

        // The discarded partial is not prepended: a later valid sequence reassembles alone.
        for (const c of chunksOf(bytes, [4, 20])) provider.deliver(c);
        expect(onMessage).toHaveBeenCalledTimes(1);
        expect(onMessage).toHaveBeenCalledWith(original);
        expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
      },
    );

    it('(3) CHUNK(last=false), a whole message, an undecodable CHUNK(last=true): one reconnect; the whole message was delivered', () => {
      const { provider, onMessage } = setup();
      const bytes = serialize(original);
      // A truncated encoding: the map header promises more than the bytes hold.
      const truncated = bytes.slice(0, bytes.length - 5);

      provider.deliver(chunk(false, truncated.slice(0, 8)));
      provider.deliver(whole);
      provider.deliver(chunk(true, truncated.slice(8)));

      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith(whole);
    });
  });

  describe('T10: an undecodable reassembled message', () => {
    it('forces a reconnect, routes nothing and leaves the buffer empty', () => {
      const { provider, onMessage } = setup();
      const bytes = serialize(original);
      // A truncated encoding: the map header promises more than the bytes hold.
      const truncated = bytes.slice(0, bytes.length - 5);
      for (const c of chunksOf(truncated, [6])) provider.deliver(c);

      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
      expect(onMessage).not.toHaveBeenCalled();

      // An empty buffer lets a whole message through with no further violation.
      provider.deliver(whole);
      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith(whole);
      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
    });
  });

  describe('T11: the reassembly cap', () => {
    it('is 1 GiB', () => {
      expect(MAX_REASSEMBLY_BYTES).toBe(1_073_741_824);
    });

    it('accepts a sequence of exactly the cap and refuses one byte more', () => {
      const exact = new ChunkReassembler(10);
      expect(exact.push(chunk(false, new Uint8Array([1, 2, 3, 4, 5, 6])))).toBeNull();
      expect(exact.push(chunk(true, new Uint8Array([7, 8, 9, 10])))).toEqual(
        new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
      );
      expect(exact.isEmpty()).toBe(true);

      const over = new ChunkReassembler(10);
      expect(over.push(chunk(false, new Uint8Array(6)))).toBeNull();
      expect(() => over.push(chunk(true, new Uint8Array(5)))).toThrow();
      expect(over.isEmpty()).toBe(true);
    });

    it('through WebSocketManager: the cap routes an exact message and reconnects one byte over', () => {
      const { provider, manager, onMessage } = setup();
      const reassemblers = (manager as unknown as { reassemblers: Map<string, ChunkReassembler> })
        .reassemblers;

      const bytes = serialize({ type: 'ACK' });
      const cap = bytes.length;

      reassemblers.set('default', new ChunkReassembler(cap));
      for (const c of chunksOf(bytes, [4])) provider.deliver(c);
      expect(onMessage).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledWith({ type: 'ACK' });
      expect(provider.forceReconnect).not.toHaveBeenCalled();

      reassemblers.set('default', new ChunkReassembler(cap - 1));
      for (const c of chunksOf(bytes, [4])) provider.deliver(c);
      expect(provider.forceReconnect).toHaveBeenCalledTimes(1);
      expect(onMessage).toHaveBeenCalledTimes(1);
    });
  });
});
