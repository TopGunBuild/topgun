/**
 * Tests for ConnectionPool socket lifetime: which socket may speak for a node.
 */
import { serialize } from '@topgunbuild/core';
import { ConnectionPool } from '../cluster/ConnectionPool';

/**
 * A socket the test drives by hand. `close()` only starts the closing
 * handshake, as a real socket does: until the peer answers, frames that were
 * already on their way are still delivered to `onmessage`.
 */
class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readyState = MockWebSocket.CONNECTING;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: unknown[] = [];

  constructor(public readonly url: string) {
    MockWebSocket.instances.push(this);
  }

  send(data: unknown): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = MockWebSocket.CLOSING;
  }

  open(): void {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  /** Delivers a server frame the way the runtime does, as a binary message event. */
  receive(message: unknown): void {
    this.onmessage?.({ data: new Uint8Array(serialize(message)).buffer });
  }
}

describe('ConnectionPool', () => {
  const realWebSocket = (globalThis as { WebSocket?: unknown }).WebSocket;

  beforeEach(() => {
    MockWebSocket.instances = [];
    (globalThis as { WebSocket?: unknown }).WebSocket = MockWebSocket;
  });

  afterEach(() => {
    (globalThis as { WebSocket?: unknown }).WebSocket = realWebSocket;
  });

  test("a frame from a socket the pool has dropped is not delivered under the node's new connection", async () => {
    const pool = new ConnectionPool({});
    const delivered: Array<[string, { type: string; payload?: unknown }]> = [];
    pool.on('message', (nodeId: string, message: { type: string; payload?: unknown }) => {
      delivered.push([nodeId, message]);
    });

    // The node is connected, then the whole pool is shut down and the same node
    // is added again under the same id: what a connection reset does.
    await pool.addNode('node-a', 'ws://node-a:8080');
    const [dropped] = MockWebSocket.instances;
    dropped.open();
    // The handler as the runtime holds it for an event it has already queued.
    const queuedDispatch = dropped.onmessage!;
    await pool.close();

    await pool.addNode('node-a', 'ws://node-a:8080');
    const current = MockWebSocket.instances[1];
    expect(current).not.toBe(dropped);
    current.open();

    // An answer the server wrote before it saw the close is still in transit on
    // the dropped socket. It belongs to the old connection: read as a frame of
    // the new one, it would answer requests it has never seen.
    dropped.receive({ type: 'OP_ACK', payload: { lastId: '2', achievedLevel: 'APPLIED' } });
    expect(delivered).toEqual([]);
    // Detaching the handler is not what the guarantee may rest on: a dispatch
    // queued before the drop reaches the handler all the same.
    queuedDispatch({
      data: new Uint8Array(serialize({ type: 'OP_ACK', payload: { lastId: '2' } })).buffer,
    });
    expect(delivered).toEqual([]);

    // The node's current socket still speaks for it.
    current.receive({ type: 'OP_ACK', payload: { lastId: '3', achievedLevel: 'APPLIED' } });
    expect(delivered).toEqual([
      ['node-a', { type: 'OP_ACK', payload: { lastId: '3', achievedLevel: 'APPLIED' } }],
    ]);

    await pool.close();
  });
});
