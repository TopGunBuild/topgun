/**
 * `ClusterClient.sendBatch` reports the frames it puts on the wire.
 *
 * A node acknowledges an `OP_BATCH` by the id of its last operation only. The
 * engine can attribute that acknowledgement to the right operations only if the
 * provider tells it which ids travelled together, once per frame a connection
 * accepted, and never for a frame that did not leave (TG-SYNC-004). The engine
 * tests drive a mock provider; these drive the real `ClusterClient` over a
 * stubbed pool, so a dropped report shows up here and not as unconfirmed writes
 * in a cluster.
 */
import { ClusterClient } from '../cluster/ClusterClient';

interface Frame {
  target: string;
  ids: string[];
}

/**
 * A real ClusterClient whose pool and router answer from the given tables
 * instead of from sockets and a partition map.
 */
function clientOver(options: {
  routingMode: 'direct' | 'forward';
  /** key -> owning node; a key that is absent has no partition-map entry. */
  owners?: Record<string, string>;
  /** Nodes whose connection is down. */
  disconnected?: string[];
  /** Targets whose hand-off is refused; 'primary' names the fallback connection. */
  refusing?: string[];
}) {
  const client = new ClusterClient({
    enabled: true,
    seedNodes: ['ws://localhost:9001'],
    routingMode: options.routingMode,
  });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the pool, the router and the routing flag are private; the test replaces what they answer, not how sendBatch uses them
  const internals = client as any;
  const owners = options.owners ?? {};
  const disconnected = new Set(options.disconnected ?? []);
  const refusing = new Set(options.refusing ?? []);
  const frames: Frame[] = [];

  const handOff = (target: string, message: { payload: { ops: Array<{ id: string }> } }) => {
    if (refusing.has(target)) return false;
    frames.push({ target, ids: message.payload.ops.map((op) => op.id) });
    return true;
  };

  jest
    .spyOn(internals.connectionPool, 'send')
    .mockImplementation((nodeId: unknown, message: unknown) =>
      handOff(nodeId as string, message as Parameters<typeof handOff>[1]),
    );
  jest
    .spyOn(internals.connectionPool, 'sendToPrimary')
    .mockImplementation((message: unknown) =>
      handOff('primary', message as Parameters<typeof handOff>[1]),
    );
  jest
    .spyOn(internals.connectionPool, 'isNodeConnected')
    .mockImplementation((nodeId: unknown) => !disconnected.has(nodeId as string));
  jest
    .spyOn(internals.partitionRouter, 'route')
    .mockImplementation((key: unknown) =>
      owners[key as string] ? { nodeId: owners[key as string] } : null,
    );
  internals.routingActive = options.routingMode === 'direct';

  return { client, frames };
}

const op = (id: string) => ({ key: `key-${id}`, message: { id, mapName: 'm', key: `key-${id}` } });

describe('ClusterClient.sendBatch reports the frames it sends', () => {
  afterEach(() => {
    jest.restoreAllMocks();
  });

  test('direct routing reports each frame a connection accepted, with that frame’s ids in order', async () => {
    const { client, frames } = clientOver({
      routingMode: 'direct',
      owners: { 'key-1': 'node-a', 'key-2': 'node-b', 'key-3': 'node-a' },
    });
    const reported: string[][] = [];

    // Ops 4 and 5 have no partition-map entry and travel together to the
    // fallback connection: a third frame, reported like the two direct ones.
    const results = client.sendBatch([op('1'), op('2'), op('3'), op('4'), op('5')], (ids) =>
      reported.push(ids),
    );

    expect(frames).toEqual([
      { target: 'node-a', ids: ['1', '3'] },
      { target: 'node-b', ids: ['2'] },
      { target: 'primary', ids: ['4', '5'] },
    ]);
    // One report per frame, each exactly the ids that frame carried.
    expect(reported).toEqual(frames.map((frame) => frame.ids));
    expect([...results.values()]).toEqual([true, true, true, true, true]);

    await client.close();
  });

  test('a frame whose hand-off was refused is not reported', async () => {
    const { client, frames } = clientOver({
      routingMode: 'direct',
      owners: { 'key-1': 'node-a', 'key-2': 'node-b', 'key-3': 'node-a' },
      refusing: ['node-b', 'primary'],
    });
    const reported: string[][] = [];

    const results = client.sendBatch([op('1'), op('2'), op('3'), op('4')], (ids) =>
      reported.push(ids),
    );

    // Only node-a took its frame. A report for the other two would let a later
    // acknowledgement retire operations that never left the client.
    expect(frames).toEqual([{ target: 'node-a', ids: ['1', '3'] }]);
    expect(reported).toEqual([['1', '3']]);
    expect(Object.fromEntries(results)).toEqual({
      'key-1': true,
      'key-2': false,
      'key-3': true,
      'key-4': false,
    });

    await client.close();
  });

  test('an op whose owner is not connected is reported in the fallback frame it travels in', async () => {
    const { client, frames } = clientOver({
      routingMode: 'direct',
      owners: { 'key-1': 'node-a', 'key-2': 'node-b' },
      disconnected: ['node-b'],
    });
    const reported: string[][] = [];

    client.sendBatch([op('1'), op('2')], (ids) => reported.push(ids));

    expect(frames).toEqual([
      { target: 'node-a', ids: ['1'] },
      { target: 'primary', ids: ['2'] },
    ]);
    expect(reported).toEqual([['1'], ['2']]);

    await client.close();
  });

  test('forward mode reports the single frame it sends, and nothing when that frame is refused', async () => {
    const accepted = clientOver({ routingMode: 'forward' });
    const reported: string[][] = [];

    accepted.client.sendBatch([op('1'), op('2'), op('3')], (ids) => reported.push(ids));

    expect(accepted.frames).toEqual([{ target: 'primary', ids: ['1', '2', '3'] }]);
    expect(reported).toEqual([['1', '2', '3']]);
    await accepted.client.close();

    const refused = clientOver({ routingMode: 'forward', refusing: ['primary'] });
    const reportedWhenRefused: string[][] = [];

    const results = refused.client.sendBatch([op('1'), op('2')], (ids) =>
      reportedWhenRefused.push(ids),
    );

    expect(refused.frames).toEqual([]);
    expect(reportedWhenRefused).toEqual([]);
    expect([...results.values()]).toEqual([false, false]);
    await refused.client.close();
  });

  test('a caller that passes no reporter still gets its frames sent', async () => {
    const { client, frames } = clientOver({
      routingMode: 'direct',
      owners: { 'key-1': 'node-a' },
    });

    const results = client.sendBatch([op('1'), op('2')]);

    expect(frames).toEqual([
      { target: 'node-a', ids: ['1'] },
      { target: 'primary', ids: ['2'] },
    ]);
    expect([...results.values()]).toEqual([true, true]);

    await client.close();
  });
});
