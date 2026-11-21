/**
 * Integration: a write burst through the TopGun client puts each write on the
 * wire once, and a live subscriber of the map stays connected through it.
 *
 * Writer W is a TopGunClient with in-memory storage and the default
 * backpressure. It writes 4 000 distinct keys with `set()` without waiting for
 * any acknowledgement, then waits until nothing is pending. Every OP_BATCH it
 * hands to its connection provider is decoded and its ops counted, so the test
 * sees exactly what reached the wire. A client that re-sent its whole
 * unacknowledged op log on every write would put about n²/2 ops there (capped
 * per write by `maxPendingOps`), and the server would fan each one out to the
 * subscriber as a SERVER_EVENT.
 *
 * "Exactly 4 000" assumes every acknowledgement of the burst arrives within
 * 10 s of its send, the client's first resend delay. A slower one fires a
 * re-send and fails the count; that is deliberate, because the count is the
 * evidence that nothing is re-sent on a healthy burst.
 *
 * - Raw subscriber: S is a protocol-level connection holding a live query of
 *   the map, in a process of its own (helpers/raw-subscriber-process.js). Its
 *   own decoding cost cannot be what disconnects it, so this case measures the
 *   writer's wire volume. It must not share the test's event loop: W's burst
 *   keeps that loop busy for seconds, during which an in-process S reads
 *   nothing and the server drops it as a slow consumer, which no real
 *   subscriber, running in a process other than the writer's, would see.
 * - Client subscriber: the map is seeded with 5 000 rows through raw OP_BATCHes,
 *   S is a TopGunClient holding a live query of it, and the burst starts once
 *   S's view holds the seed. S's view must end with all 9 000 keys and S must
 *   never leave CONNECTED.
 *
 * In both cases the server's log must not contain the slow-consumer line, which
 * it writes when it disconnects a subscriber that cannot keep up.
 */

import * as child_process from 'child_process';
import * as path from 'path';

import { deserialize } from '@topgunbuild/core';
import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { StateChangeEvent } from '@topgunbuild/client';

import { spawnRustServer, createTestToken, MemoryStorageAdapter, SpawnedServer } from './helpers';
import { TestClient } from './helpers/test-client';
import { connectWithin, firstFrame } from './helpers/two-binary';

const BURST_KEYS = 4_000;
const SEED_ROWS = 5_000;
const SEED_BATCH = 100;
const DRAIN_CEILING_MS = 60_000;
const VIEW_CEILING_MS = 60_000;
const STEP_WAIT_MS = 10_000;
const SLOW_CONSUMER_TEXT = 'slow consumer exceeded broadcast drop threshold';

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

async function until(what: string, probe: () => boolean, ceilingMs: number): Promise<void> {
  const deadline = Date.now() + ceilingMs;
  for (;;) {
    if (probe()) return;
    if (Date.now() >= deadline) throw new Error(`${what}: not within ${ceilingMs} ms`);
    await sleep(25);
  }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test reaches into the client's engine
const engineOf = (client: TopGunClient): any => (client as any).syncEngine;

function makeClient(port: number, userId: string): TopGunClient {
  const token = createTestToken(userId, ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
  });
}

/** Counts the ops of every OP_BATCH the client hands to its connection provider. */
function countOutboundOps(client: TopGunClient): { total: () => number; ids: Set<string> } {
  const ids = new Set<string>();
  let total = 0;
  const provider = engineOf(client).webSocketManager.getConnectionProvider();
  const send = provider.send.bind(provider);
  provider.send = (data: ArrayBuffer | Uint8Array, key?: string) => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
    const message = deserialize(data instanceof Uint8Array ? data : new Uint8Array(data)) as any;
    if (message?.type === 'OP_BATCH') {
      for (const op of message.payload?.ops ?? []) {
        total += 1;
        ids.add(String(op.id));
      }
    }
    return send(data, key);
  };
  return { total: () => total, ids };
}

/** Starts the client and waits until it is CONNECTED. */
async function connected(client: TopGunClient, who: string): Promise<void> {
  await client.start();
  await until(
    `${who} connects`,
    () => client.getConnectionState() === SyncState.CONNECTED,
    STEP_WAIT_MS,
  );
}

/**
 * Writes BURST_KEYS keys through W without awaiting anything, then waits until
 * every write has been on the wire and W has nothing pending. The pending count
 * alone is not enough: a write enters the op log only once its local commit
 * resolves, so right after the loop nothing is pending yet.
 */
async function burst(
  w: TopGunClient,
  mapName: string,
  wire: ReturnType<typeof countOutboundOps>,
): Promise<void> {
  const map = w.getMap<string, { v: number }>(mapName);
  for (let i = 0; i < BURST_KEYS; i++) map.set(`w-${i}`, { v: i });
  await until(
    'W puts every write on the wire and drains its burst',
    () => wire.ids.size === BURST_KEYS && w.getPendingOpsCount() === 0,
    DRAIN_CEILING_MS,
  );
}

describe('Integration: a client write burst is sent once per write', () => {
  let server: SpawnedServer | null = null;
  let log = '';
  const clients: TopGunClient[] = [];
  const raws: TestClient[] = [];
  const children: child_process.ChildProcess[] = [];

  beforeEach(async () => {
    log = '';
    server = await spawnRustServer();
    server.process.stdout?.on('data', (chunk: Buffer) => {
      log += chunk.toString('utf8');
    });
  });

  afterEach(async () => {
    for (const client of clients.splice(0)) await client.close().catch(() => undefined);
    for (const raw of raws.splice(0)) raw.close();
    for (const child of children.splice(0)) child.kill();
    await server?.cleanup().catch(() => undefined);
    server = null;
  });

  test('a raw live subscriber stays connected; the writer puts 4 000 ops on the wire', async () => {
    const port = server!.port;
    const mapName = 'burst_raw_m';

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IPC messages of the subscriber process
    const fromS: any[] = [];
    const s = child_process.fork(
      path.join(__dirname, 'helpers', 'raw-subscriber-process.js'),
      [String(port), mapName],
      { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] },
    );
    children.push(s);
    s.on('message', (message) => fromS.push(message));
    const said = (type: string) => fromS.find((m) => m.type === type);
    await until('S holds its live query', () => said('ready') !== undefined, STEP_WAIT_MS);

    const w = makeClient(port, 'burst-raw-writer');
    clients.push(w);
    await connected(w, 'W');
    const wire = countOutboundOps(w);

    await burst(w, mapName, wire);

    expect(wire.ids.size).toBe(BURST_KEYS);
    expect(wire.total()).toBe(BURST_KEYS);
    s.send({ type: 'final' });
    await until(
      'S answers with a final query of the map',
      () => said('final') !== undefined || said('closed') !== undefined,
      STEP_WAIT_MS,
    );
    expect(said('closed')).toBeUndefined();
    expect(log.includes(SLOW_CONSUMER_TEXT)).toBe(false);
    expect(said('final').events).toBe(BURST_KEYS);
    expect(said('final').keys).toBe(BURST_KEYS);
  }, 120_000);

  test('a client holding a 5 000-row live query stays connected and sees every key', async () => {
    const port = server!.port;
    const mapName = 'burst_view_m';

    // Seed through raw OP_BATCHes, one acknowledged batch at a time.
    const seeder = await connectWithin(port, 'burst-seeder', STEP_WAIT_MS);
    raws.push(seeder);
    for (let at = 0; at < SEED_ROWS; at += SEED_BATCH) {
      const ops = [];
      for (let i = at; i < Math.min(at + SEED_BATCH, SEED_ROWS); i++) {
        ops.push({
          id: `seed-${i}`,
          mapName,
          opType: 'PUT',
          key: `s-${i}`,
          record: {
            value: { v: i },
            timestamp: { millis: Date.now(), counter: i, nodeId: 'seed' },
          },
        });
      }
      seeder.messages.length = 0;
      seeder.send({ type: 'OP_BATCH', payload: { ops } });
      const frame = await firstFrame(seeder, ['OP_ACK', 'OP_REJECTED', 'ERROR'], STEP_WAIT_MS);
      const lastId = ops[ops.length - 1].id;
      if (frame?.type !== 'OP_ACK' || frame.payload?.lastId !== lastId) {
        throw new Error(`seed batch ending ${lastId}: ${JSON.stringify(frame)?.slice(0, 400)}`);
      }
    }

    const s = makeClient(port, 'burst-view-subscriber');
    clients.push(s);
    await connected(s, 'S');
    const stateChanges: StateChangeEvent[] = [];
    s.onConnectionStateChange((event) => stateChanges.push(event));
    let view: Array<{ _key: string }> = [];
    s.query<{ v: number }>(mapName, {}).subscribe((rows) => {
      view = rows as Array<{ _key: string }>;
    });
    await until("S's view holds the seed", () => view.length === SEED_ROWS, VIEW_CEILING_MS);

    const w = makeClient(port, 'burst-view-writer');
    clients.push(w);
    await connected(w, 'W');
    const wire = countOutboundOps(w);

    await burst(w, mapName, wire);

    expect(wire.ids.size).toBe(BURST_KEYS);
    expect(wire.total()).toBe(BURST_KEYS);
    await until(
      "S's view holds every key",
      () => view.length === SEED_ROWS + BURST_KEYS,
      VIEW_CEILING_MS,
    );
    expect(new Set(view.map((r) => r._key)).size).toBe(SEED_ROWS + BURST_KEYS);
    expect(log.includes(SLOW_CONSUMER_TEXT)).toBe(false);
    expect(stateChanges.filter((e) => e.to !== SyncState.CONNECTED)).toEqual([]);
  }, 180_000);
});
