/**
 * A live socket closes at its token's expiry, and an in-band refresh for the
 * same principal keeps it open (TG-AUTH-001).
 *
 * The server's JWT leeway is 60 s, so a token whose `exp` is 55 s in the past
 * is admitted and its socket closes about 5 s later. The client here is the
 * raw protocol client: it never refreshes unless the test sends an `AUTH`
 * itself, as an old client would.
 *
 * - an expired socket gets `AUTH_FAIL` code 4401 and the close within
 *   5 s ± 1.5 s, and `topgun_ws_token_expiry_closes_total` rises by one from
 *   the zero of the first scrape;
 * - with `TOPGUN_CLOSE_ON_TOKEN_EXPIRY=false` the same socket stays open;
 * - a refresh for the same principal is acknowledged and outlives the old
 *   deadline; one with changed roles gets 4409 and the close; an `AUTH` inside
 *   a `BATCH` envelope changes nothing;
 * - on a NO_AUTH server an `AUTH` frame gets no reply and the socket serves on.
 */

import * as jwt from 'jsonwebtoken';
import { serialize } from '@topgunbuild/core';
import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';

const JWT_SECRET = 'test-e2e-secret';
const CLOSES = 'topgun_ws_token_expiry_closes_total';

/** A token for `sub` with `roles` whose `exp` is `expIn` seconds from now. */
function token(sub: string, roles: string[], expIn: number): string {
  return jwt.sign({ sub, roles, exp: Math.floor(Date.now() / 1000) + expIn }, JWT_SECRET);
}

/** The counter's value on `/metrics`; `null` when the series is absent. */
async function closes(port: number): Promise<number | null> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  const line = text.split('\n').find((l) => l.startsWith(`${CLOSES} `));
  return line ? Number(line.slice(CLOSES.length + 1)) : null;
}

interface Session {
  client: TestClient;
  /** When the socket closed (ms since the epoch); `undefined` while open. */
  closedAt: () => number | undefined;
}

/** Opens a raw socket; with `tok`, authenticates with it and waits for the ack. */
async function open(port: number, tok?: string): Promise<Session> {
  const client = await createRustTestClient(port, { autoAuth: false });
  let closedAt: number | undefined;
  client.ws.on('close', () => {
    closedAt = Date.now();
  });
  if (tok !== undefined) {
    await client.waitForMessage('AUTH_REQUIRED', 8000);
    client.send({ type: 'AUTH', token: tok });
    await client.waitForMessage('AUTH_ACK', 8000);
  }
  return { client, closedAt: () => closedAt };
}

/** Frames of `type` the client received from index `from` on. */
function received(client: TestClient, type: string, from = 0): any[] {
  return client.messages.slice(from).filter((m) => m.type === type);
}

/** Whether the socket still answers a PING. */
async function answers(client: TestClient): Promise<boolean> {
  const from = client.messages.length;
  client.send({ type: 'PING', timestamp: Date.now() });
  try {
    await waitUntil(() => received(client, 'PONG', from).length > 0, 3000);
    return true;
  } catch {
    return false;
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Hand-encodes `fixmap{ type:"BATCH", count:1, data:<bin32> }` around one inner message. */
function batchOf(inner: Uint8Array): Uint8Array {
  const len = (n: number) => {
    const b = Buffer.alloc(4);
    b.writeUInt32BE(n, 0);
    return b;
  };
  const data = Buffer.concat([len(inner.length), Buffer.from(inner)]);
  return new Uint8Array(
    Buffer.concat([
      Buffer.from([0x83, 0xa4]),
      Buffer.from('type'),
      Buffer.from([0xa5]),
      Buffer.from('BATCH'),
      Buffer.from([0xa5]),
      Buffer.from('count'),
      Buffer.from([1, 0xa4]),
      Buffer.from('data'),
      Buffer.from([0xc6]),
      len(data.length),
      data,
    ]),
  );
}

describe('Integration: sockets close at token expiry (TG-AUTH-001)', () => {
  const servers: SpawnedServer[] = [];
  const clients: TestClient[] = [];

  afterAll(async () => {
    clients.forEach((c) => c.close());
    await Promise.all(servers.map((s) => s.cleanup()));
  });

  async function server(env: Record<string, string> = {}): Promise<SpawnedServer> {
    const s = await spawnRustServer({ env });
    servers.push(s);
    return s;
  }

  test('an expired socket is closed with 4401', async () => {
    const { port } = await server();
    expect(await closes(port)).toBe(0);

    const session = await open(port, token('old-client', ['USER'], -55));
    clients.push(session.client);
    const ackAt = Date.now();
    await waitUntil(() => received(session.client, 'AUTH_FAIL').length > 0, 10_000, 20);
    const failAt = Date.now();
    await waitUntil(() => session.closedAt() !== undefined, 5000, 20);

    // The server closes right behind the AUTH_FAIL, not when its teardown
    // gives up on the writer two seconds later.
    expect(session.closedAt()! - failAt).toBeLessThan(1000);
    const fail = received(session.client, 'AUTH_FAIL');
    expect(fail).toHaveLength(1);
    expect(fail[0].code).toBe(4401);
    const after = session.closedAt()! - ackAt;
    expect(after).toBeGreaterThanOrEqual(3500);
    expect(after).toBeLessThanOrEqual(6500);
    expect(await closes(port)).toBe(1);
  });

  test('the closing can be turned off', async () => {
    const { port } = await server({ TOPGUN_CLOSE_ON_TOKEN_EXPIRY: 'false' });
    const session = await open(port, token('old-client', ['USER'], -55));
    clients.push(session.client);

    await sleep(13_000);
    expect(session.closedAt()).toBeUndefined();
    expect(received(session.client, 'AUTH_FAIL')).toHaveLength(0);
    expect(await answers(session.client)).toBe(true);
    expect(await closes(port)).toBe(0);
  });

  test('a refresh with changed roles closes the socket', async () => {
    const { port } = await server();
    const session = await open(port, token('refresher', ['USER', 'editor'], -55));
    clients.push(session.client);
    const { client } = session;

    // Same principal, roles in another order: acknowledged, no device token,
    // and the socket outlives the first token's deadline.
    client.send({ type: 'AUTH', token: token('refresher', ['editor', 'USER'], 3600) });
    await waitUntil(() => received(client, 'AUTH_ACK').length === 2, 5000);
    const ack = received(client, 'AUTH_ACK')[1];
    expect(ack.userId).toBe('refresher');
    expect(ack.deviceToken).toBeUndefined();

    // An AUTH for another principal inside a BATCH envelope changes nothing.
    client.ws.send(batchOf(serialize({ type: 'AUTH', token: token('intruder', ['USER'], 3600) })));

    await sleep(8000);
    expect(session.closedAt()).toBeUndefined();
    expect(received(client, 'AUTH_FAIL')).toHaveLength(0);
    expect(await answers(client)).toBe(true);

    client.send({ type: 'AUTH', token: token('refresher', ['USER', 'admin'], 3600) });
    await waitUntil(() => session.closedAt() !== undefined, 5000);
    const fail = received(client, 'AUTH_FAIL');
    expect(fail).toHaveLength(1);
    expect(fail[0].code).toBe(4409);
  });

  test('an AUTH on a NO_AUTH server is ignored', async () => {
    const { port } = await server({ TOPGUN_NO_AUTH: 'true' });
    const session = await open(port);
    clients.push(session.client);
    const { client } = session;

    client.send({ type: 'AUTH', token: 'not-a-token' });
    await sleep(1000);
    expect(client.messages).toHaveLength(0);
    expect(session.closedAt()).toBeUndefined();

    client.send({
      type: 'OP_BATCH',
      payload: {
        ops: [
          {
            id: 'after-auth',
            mapName: 'no-auth.m',
            opType: 'PUT',
            key: 'k',
            record: createLWWRecord({ v: 1 }),
          },
        ],
      },
    });
    await waitUntil(() => received(client, 'OP_ACK').length === 1, 5000);
  });
});
