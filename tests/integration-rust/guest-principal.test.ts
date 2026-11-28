/**
 * Guest principals over the wire (TG-AUTHZ-002, TG-AUTHZ-005, TG-AUTH-002).
 *
 * One server with guest principals on (`TOPGUN_GUEST_CLAIM=tg_guest`) and two
 * policies: an allow of everything, and a deny of every write whose caller has
 * `auth.kind == 'guest'`. The app backend mints the tokens; here the test
 * signs them with the server's secret.
 *
 * - a guest's write is refused on WebSocket and on HTTP `/sync`, also when its
 *   value carries `{auth: {kind: 'user'}}`: the kind comes from the token,
 *   never from the record;
 * - a user's write to the same map is acknowledged on both paths.
 *
 * A second server, on a durable store, proves the lifetime of a guest's device
 * credential: it re-binds within the server process and is replaced after a
 * restart, while a user's credential survives the restart.
 *
 * A third drives real TopGunClients, each in a process of its own
 * (helpers/auth-client.ts): a guest that switches to a user reconnects as the
 * user, a guest write the user's policies refuse is reported on
 * `onWriteRejected`, and two storage-less guest tabs of one visitor stay
 * independent when one of them logs in.
 */

import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import * as os from 'os';
import * as path from 'path';
import { serialize, deserialize } from '@topgunbuild/core';
import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  createLWWRecord,
  waitUntil,
  SpawnedServer,
  TestClient,
} from './helpers';
import { AuthClient } from './helpers/auth-client';

const JWT_SECRET = 'test-e2e-secret';
const GUEST_CLAIM = 'tg_guest';
const MAP = 'shop.cart';
const adminToken = createTestToken('admin-setup', ['admin']);

function token(sub: string, guest: boolean): string {
  return jwt.sign({ sub, roles: ['USER'], [GUEST_CLAIM]: guest }, JWT_SECRET, {
    expiresIn: '1h',
  });
}

async function connect(port: number, sub: string, guest: boolean): Promise<TestClient> {
  const client = await createRustTestClient(port, { userId: sub, autoAuth: false });
  await client.waitForMessage('AUTH_REQUIRED', 8000);
  client.send({ type: 'AUTH', token: token(sub, guest) });
  await client.waitForMessage('AUTH_ACK', 8000);
  return client;
}

async function createPolicy(port: number, policy: Record<string, string>): Promise<void> {
  const resp = await fetch(`http://localhost:${port}/api/admin/policies`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${adminToken}` },
    body: JSON.stringify(policy),
  });
  if (resp.status !== 201) {
    throw new Error(`policy ${policy.id}: HTTP ${resp.status} ${await resp.text()}`);
  }
}

/** One OP_BATCH write; resolves with OP_ACK, OP_REJECTED or ERROR. */
async function write(
  client: TestClient,
  key: string,
  value: unknown,
): Promise<{ type: string; payload?: any }> {
  const from = client.messages.length;
  client.send({
    type: 'OP_BATCH',
    payload: {
      ops: [
        {
          id: `op-${key}-${Math.random().toString(36).slice(2)}`,
          mapName: MAP,
          opType: 'PUT',
          key,
          record: createLWWRecord(value),
        },
      ],
    },
  });
  let reply: { type: string; payload?: any } | undefined;
  await waitUntil(() => {
    reply = client.messages
      .slice(from)
      .find((m) => ['OP_ACK', 'OP_REJECTED', 'ERROR'].includes(m.type));
    return reply !== undefined;
  }, 5000);
  return reply!;
}

/** POST /sync of one write with `bearer`; returns the status and decoded body. */
async function syncWrite(port: number, bearer: string, id: string, value: unknown): Promise<any> {
  const resp = await fetch(`http://127.0.0.1:${port}/sync`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/msgpack', Authorization: `Bearer ${bearer}` },
    body: Buffer.from(
      serialize({
        clientId: 'guest-http',
        clientHlc: { millis: Date.now(), counter: 0, nodeId: 'guest-http' },
        operations: [{ id, mapName: MAP, opType: 'PUT', key: id, record: createLWWRecord(value) }],
      }),
    ),
  });
  return {
    status: resp.status,
    body: resp.ok ? deserialize(new Uint8Array(await resp.arrayBuffer())) : null,
  };
}

describe('Integration: guest principals (TOPGUN_GUEST_CLAIM)', () => {
  let server: SpawnedServer;
  const open: TestClient[] = [];

  beforeAll(async () => {
    server = await spawnRustServer({ env: { TOPGUN_GUEST_CLAIM: GUEST_CLAIM } });
    await createPolicy(server.port, {
      id: 'allow-all',
      mapPattern: '**',
      action: 'all',
      effect: 'allow',
    });
    await createPolicy(server.port, {
      id: 'no-guest-writes',
      mapPattern: '**',
      action: 'write',
      effect: 'deny',
      conditionExpr: "auth.kind == 'guest'",
    });
  });

  afterAll(async () => {
    open.forEach((c) => c.close());
    await server.cleanup();
  });

  test('a guest is held to kind-conditioned rules', async () => {
    const visitor = await connect(server.port, 'g-visitor', true);
    const user = await connect(server.port, 'u-alice', false);
    open.push(visitor, user);

    for (const value of [{ item: 'socks' }, { item: 'socks', auth: { kind: 'user' } }]) {
      const reply = await write(visitor, 'g1', value);
      expect(reply.type).toBe('OP_REJECTED');
      expect(reply.payload?.code).toBe(403);
    }
    expect((await write(user, 'u1', { item: 'shoes' })).type).toBe('OP_ACK');

    const guestSync = await syncWrite(server.port, token('g-visitor', true), 'hg', {
      item: 'socks',
      auth: { kind: 'user' },
    });
    expect(guestSync.status).toBe(200);
    expect((guestSync.body.errors ?? []).map((e: any) => [e.code, e.context])).toEqual([
      [403, 'hg'],
    ]);

    const userSync = await syncWrite(server.port, token('u-alice', false), 'hu', { item: 'hat' });
    expect(userSync.status).toBe(200);
    expect(userSync.body.errors ?? []).toEqual([]);
  });
});

describe('Integration: guest device identity (TG-AUTH-002)', () => {
  let dir = '';
  let server: SpawnedServer | null = null;

  const start = async (): Promise<void> => {
    server = await spawnRustServer({
      env: {
        TOPGUN_GUEST_CLAIM: GUEST_CLAIM,
        STORAGE_BACKEND: 'rocksdb',
        TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
      },
    });
  };

  /** A clean stop (exit 0), so the user's credential row is stored, then a new process. */
  const restart = async (): Promise<void> => {
    const stopping = server!;
    server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      20_000,
    );
    expect(stopping.process.exitCode).toBe(0);
    await start();
  };

  /** One connection that authenticates, presenting `deviceToken` when given; returns its AUTH_ACK. */
  async function auth(sub: string, guest: boolean, deviceToken?: string): Promise<any> {
    const client = await createRustTestClient(server!.port, { userId: sub, autoAuth: false });
    try {
      await client.waitForMessage('AUTH_REQUIRED', 8000);
      client.send({
        type: 'AUTH',
        token: token(sub, guest),
        ...(deviceToken ? { deviceToken } : {}),
      });
      return await client.waitForMessage('AUTH_ACK', 8000);
    } finally {
      client.close();
    }
  }

  beforeAll(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-guest-device-'));
    await start();
  });

  afterAll(async () => {
    if (server) await server.cleanup();
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('guest device identity lives as long as the server process', async () => {
    const guest = await auth('g-device', true);
    expect(guest.deviceToken).toMatch(/^[0-9a-f-]{36}\.[0-9a-f]{64}$/);
    const user = await auth('u-device', false);
    expect(user.deviceToken).toBeDefined();

    const reconnected = await auth('g-device', true, guest.deviceToken);
    expect([reconnected.deviceId, reconnected.deviceToken]).toEqual([guest.deviceId, undefined]);

    await restart();

    const afterRestart = await auth('g-device', true, guest.deviceToken);
    expect(afterRestart.deviceId).not.toBe(guest.deviceId);
    expect(afterRestart.deviceToken).toMatch(/^[0-9a-f-]{36}\.[0-9a-f]{64}$/);
    const userAfterRestart = await auth('u-device', false, user.deviceToken);
    expect([userAfterRestart.deviceId, userAfterRestart.deviceToken]).toEqual([
      user.deviceId,
      undefined,
    ]);
  });
});

describe('Integration: a client switching from guest to user (TG-AUTH-001)', () => {
  const GUEST_ONLY = 'acme.visitor';
  const USER_ONLY = 'acme.conv';
  let dir = '';
  let port = 0;
  let server: SpawnedServer | null = null;
  const clients: AuthClient[] = [];

  /** Guests may write only `acme.visitor.**`, users only `acme.conv.**`. */
  const start = async (fixedPort?: number): Promise<void> => {
    server = await spawnRustServer({
      port: fixedPort,
      env: {
        TOPGUN_GUEST_CLAIM: GUEST_CLAIM,
        STORAGE_BACKEND: 'rocksdb',
        TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
      },
    });
    port = server.port;
  };

  const stop = async (): Promise<void> => {
    const stopping = server!;
    server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      20_000,
    );
  };

  const client = (startWith: { provider: string } | { token: string }): AuthClient => {
    const c = new AuthClient(port, startWith);
    clients.push(c);
    return c;
  };

  const connected = (c: AuthClient, ms = 10_000) =>
    c.until('CONNECTED', (s) => s.state === 'CONNECTED', ms);

  /** Resolves once the client has one more OP_ACK than `before` and nothing pending. */
  const acked = (c: AuthClient, before: number) =>
    c.until('the write acknowledged', (s) => s.opAcks > before && s.pending === 0, 10_000);

  beforeAll(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-guest-switch-'));
    await start();
    await createPolicy(port, { id: 'allow-all', mapPattern: '**', action: 'all', effect: 'allow' });
    await createPolicy(port, {
      id: 'guests-write-visitor-maps-only',
      mapPattern: `${USER_ONLY}.**`,
      action: 'write',
      effect: 'deny',
      conditionExpr: "auth.kind == 'guest'",
    });
    await createPolicy(port, {
      id: 'users-do-not-write-visitor-maps',
      mapPattern: `${GUEST_ONLY}.**`,
      action: 'write',
      effect: 'deny',
      conditionExpr: "auth.kind == 'user'",
    });
  });

  afterAll(async () => {
    await Promise.all(clients.map((c) => c.exit()));
    if (server) await server.cleanup();
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('a guest switching to a user reconnects', async () => {
    for (const how of ['setProvider', 'setToken'] as const) {
      const c = client({ provider: token('g1', true) });
      await connected(c);
      const from = c.status.changes.length;
      const userToken = token('u1', false);
      const switchedAt = Date.now();
      c.send({ type: how, token: userToken });

      const s = await c.until(
        `${how}: CONNECTED as the user`,
        (st) =>
          st.state === 'CONNECTED' &&
          st.changes.length > from &&
          st.auths[st.auths.length - 1]?.token === userToken,
        3_000,
      );
      const after = s.changes.slice(from);
      expect(after.map((ch) => ch.to)).toEqual([
        'DISCONNECTED',
        'CONNECTING',
        'AUTHENTICATING',
        'SYNCING',
        'CONNECTED',
      ]);
      expect(after[1].at - switchedAt).toBeLessThanOrEqual(2_000);
      expect(after[4].at - switchedAt).toBeLessThanOrEqual(3_000);
      expect(s.sockets).toBe(2);

      // Only a user may write here, so the acknowledgement names the new principal.
      c.send({ type: 'write', map: `${USER_ONLY}.u1`, key: how, value: { text: 'hello' } });
      const written = await acked(c, s.opAcks);
      expect(written.rejections).toEqual([]);
      expect(written.fails).toEqual([]);
    }
  });

  test('guest tabs are independent', async () => {
    const tabA = client({ provider: token('g-tab', true) });
    const tabB = client({ provider: token('g-tab', true) });
    const [a, b] = await Promise.all([connected(tabA), connected(tabB)]);
    await tabA.until('a device id', (s) => s.deviceId !== null, 5_000);
    const deviceB = (await tabB.until('a device id', (s) => s.deviceId !== null, 5_000)).deviceId;
    expect(tabA.status.deviceId).not.toBe(deviceB);
    expect(a.sockets).toBe(1);
    expect(b.sockets).toBe(1);

    const userToken = token('u-tab', false);
    tabA.send({ type: 'setProvider', token: userToken });
    await tabA.until(
      'tab A CONNECTED as the user',
      (s) => s.state === 'CONNECTED' && s.auths[s.auths.length - 1]?.token === userToken,
      5_000,
    );

    const before = (await tabB.fresh()).opAcks;
    tabB.send({ type: 'write', map: `${GUEST_ONLY}.g-tab.notes`, key: 'b', value: { text: 'hi' } });
    const written = await acked(tabB, before);
    expect(written.state).toBe('CONNECTED');
    expect(written.sockets).toBe(1);
    expect(written.changes.map((ch) => ch.to)).not.toContain('DISCONNECTED');
    expect(written.rejections).toEqual([]);
    expect(written.deviceId).toBe(deviceB);
  });

  test('refused guest writes are reported after login', async () => {
    const c = client({ provider: token('g1', true) });
    await connected(c);
    const fixedPort = port;

    await stop();
    await c.until('the socket lost', (s) => s.state !== 'CONNECTED', 10_000);
    c.send({ type: 'write', map: `${GUEST_ONLY}.g1.notes`, key: 'n1', value: { text: 'mine' } });
    await c.until('the write pending', (s) => s.pending === 1, 5_000);
    const userToken = token('u1', false);
    c.send({ type: 'setProvider', token: userToken });
    // Set while the server is down, the provider changes no connection: the
    // client goes on with its reconnect attempts.
    const whileDown = await c.fresh();
    expect(whileDown.state).not.toBe('CONNECTED');

    await start(fixedPort);
    const s = await c.until(
      'the refused guest write reported',
      (st) => st.state === 'CONNECTED' && st.rejections.length > 0,
      45_000,
    );
    expect(s.auths[s.auths.length - 1].token).toBe(userToken);
    expect(s.rejections).toEqual([
      expect.objectContaining({ mapName: `${GUEST_ONLY}.g1.notes`, key: 'n1', cause: 'forbidden' }),
    ]);
    expect(s.pending).toBe(0);
  }, 120_000);
});
