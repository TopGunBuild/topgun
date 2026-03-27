/**
 * Headless seed script that populates a TopGun server with demo data.
 *
 * Connects via WebSocket, writes 170 records across 3 maps (tasks, users,
 * messages) using the same MsgPack wire protocol as the real client, waits
 * for OP_ACK before proceeding to each map, then exits 0 on success.
 *
 * Required env:
 *   TOPGUN_HOST  — server hostname (default: "server")
 *   TOPGUN_PORT  — server port     (default: "8080")
 *
 * Optional env:
 *   JWT_SECRET   — if set, authenticates before writing (not needed when
 *                  the server runs with require_auth: false)
 */

import { pack, unpack } from 'msgpackr';
import WebSocket from 'ws';

// ---------------------------------------------------------------------------
// Minimal MsgPack helpers — avoids importing the full @topgunbuild/core build
// ---------------------------------------------------------------------------

function serialize(data: unknown): Uint8Array {
  return pack(stripUndefined(data));
}

function deserialize<T = unknown>(data: Uint8Array | Buffer | ArrayBuffer): T {
  const buf = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
  return coerceBigInts(unpack(buf as Uint8Array)) as T;
}

function stripUndefined(value: unknown): unknown {
  if (value === undefined) return null;
  if (Array.isArray(value)) return value.map(stripUndefined);
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      if (v !== undefined) out[k] = stripUndefined(v);
    }
    return out;
  }
  return value;
}

function coerceBigInts(value: unknown): unknown {
  if (typeof value === 'bigint') return Number(value);
  if (Array.isArray(value)) return value.map(coerceBigInts);
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = coerceBigInts(v);
    }
    return out;
  }
  return value;
}

// ---------------------------------------------------------------------------
// HLC — Hybrid Logical Clock
// Produces monotonically increasing timestamps with nodeId = "seed-client"
// ---------------------------------------------------------------------------

const NODE_ID = 'seed-client';
let hlcMillis = Date.now();
let hlcCounter = 0;

function nextTimestamp(): { millis: number; counter: number; nodeId: string } {
  const now = Date.now();
  if (now > hlcMillis) {
    hlcMillis = now;
    hlcCounter = 0;
  } else {
    hlcCounter += 1;
  }
  return { millis: hlcMillis, counter: hlcCounter, nodeId: NODE_ID };
}

function makeLWWRecord<T>(value: T): { value: T; timestamp: { millis: number; counter: number; nodeId: string } } {
  return { value, timestamp: nextTimestamp() };
}

// ---------------------------------------------------------------------------
// WebSocket client
// ---------------------------------------------------------------------------

interface Message {
  type: string;
  payload?: unknown;
  [key: string]: unknown;
}

async function connect(url: string): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    ws.binaryType = 'nodebuffer';
    ws.on('open', () => resolve(ws));
    ws.on('error', reject);
  });
}

function waitForMessage(ws: WebSocket, type: string, timeoutMs = 10_000): Promise<Message> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.removeListener('message', handler);
      reject(new Error(`Timeout waiting for ${type}`));
    }, timeoutMs);

    function handler(data: WebSocket.RawData): void {
      try {
        const msg = deserialize<Message>(data as Buffer);
        if (msg.type === type) {
          clearTimeout(timer);
          ws.removeListener('message', handler);
          resolve(msg);
        }
      } catch {
        // ignore parse errors on unrelated messages
      }
    }

    ws.on('message', handler);
  });
}

function send(ws: WebSocket, message: unknown): void {
  ws.send(serialize(message));
}

// ---------------------------------------------------------------------------
// Authenticate if JWT_SECRET is set
// ---------------------------------------------------------------------------

async function authenticate(ws: WebSocket): Promise<void> {
  const secret = process.env.JWT_SECRET;
  if (!secret) return; // auth not required

  // Wait for AUTH_REQUIRED challenge from server
  try {
    await waitForMessage(ws, 'AUTH_REQUIRED', 3_000);
  } catch {
    // Server did not send AUTH_REQUIRED — auth may not be required
    return;
  }

  // Build a minimal JWT manually to avoid adding jsonwebtoken dependency
  // when JWT_SECRET is set at runtime (Docker deployments with auth enabled)
  const header = Buffer.from(JSON.stringify({ alg: 'HS256', typ: 'JWT' })).toString('base64url');
  const claims = Buffer.from(
    JSON.stringify({ sub: 'seed-client', roles: ['ADMIN'], iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 3600 })
  ).toString('base64url');
  const sigInput = `${header}.${claims}`;

  const { createHmac } = await import('crypto');
  const sig = createHmac('sha256', secret).update(sigInput).digest('base64url');
  const token = `${sigInput}.${sig}`;

  send(ws, { type: 'AUTH', token });
  await waitForMessage(ws, 'AUTH_ACK', 5_000);
}

// ---------------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------------

type Op = {
  id: string;
  mapName: string;
  opType: 'PUT';
  key: string;
  record: ReturnType<typeof makeLWWRecord>;
};

async function seedMap(ws: WebSocket, mapName: string, ops: Op[]): Promise<void> {
  send(ws, {
    type: 'OP_BATCH',
    payload: {
      ops,
    },
  });

  await waitForMessage(ws, 'OP_ACK', 10_000);
}

// ---------------------------------------------------------------------------
// Demo data generators
// ---------------------------------------------------------------------------

const STATUSES = ['todo', 'in-progress', 'done', 'blocked'];
const PRIORITIES = ['low', 'medium', 'high', 'critical'];
const ASSIGNEES = ['alice', 'bob', 'carol', 'dave', 'eve'];
const ROLES = ['admin', 'developer', 'designer', 'manager', 'viewer'];
const CHANNELS = ['general', 'engineering', 'design', 'random', 'announcements'];

function makeTasks(mapName: string): Op[] {
  const ops: Op[] = [];
  for (let i = 1; i <= 50; i++) {
    ops.push({
      id: `seed-task-${i}`,
      mapName,
      opType: 'PUT',
      key: `task-${String(i).padStart(3, '0')}`,
      record: makeLWWRecord({
        title: `Demo Task ${i}: Implement feature ${String.fromCharCode(64 + (i % 26) + 1)}`,
        status: STATUSES[i % STATUSES.length],
        assignee: ASSIGNEES[i % ASSIGNEES.length],
        priority: PRIORITIES[i % PRIORITIES.length],
        createdAt: new Date(Date.now() - i * 60_000).toISOString(),
      }),
    });
  }
  return ops;
}

function makeUsers(mapName: string): Op[] {
  const ops: Op[] = [];
  const names = ['Alice', 'Bob', 'Carol', 'Dave', 'Eve', 'Frank', 'Grace', 'Hank',
                 'Iris', 'Jack', 'Karen', 'Leo', 'Mia', 'Ned', 'Olivia', 'Paul',
                 'Quinn', 'Rita', 'Sam', 'Tara'];
  for (let i = 0; i < 20; i++) {
    const name = names[i];
    ops.push({
      id: `seed-user-${i + 1}`,
      mapName,
      opType: 'PUT',
      key: `user-${String(i + 1).padStart(3, '0')}`,
      record: makeLWWRecord({
        name,
        email: `${name.toLowerCase()}@demo.topgunbuild.com`,
        role: ROLES[i % ROLES.length],
        avatar: `https://api.dicebear.com/7.x/avataaars/svg?seed=${name}`,
        createdAt: new Date(Date.now() - i * 3_600_000).toISOString(),
      }),
    });
  }
  return ops;
}

function makeMessages(mapName: string): Op[] {
  const ops: Op[] = [];
  const texts = [
    'Hey team, just pushed the latest changes',
    'Can someone review my PR?',
    'Build is green on main',
    'Deploying to staging now',
    'Found a bug in the sync logic',
    'Fixed the race condition — merging',
    'Performance looks great on load test',
    'Added CRDT benchmarks',
    'Docs updated for the new API',
    'Weekly sync at 3pm',
  ];
  for (let i = 1; i <= 100; i++) {
    ops.push({
      id: `seed-msg-${i}`,
      mapName,
      opType: 'PUT',
      key: `msg-${String(i).padStart(4, '0')}`,
      record: makeLWWRecord({
        sender: ASSIGNEES[i % ASSIGNEES.length],
        channel: CHANNELS[i % CHANNELS.length],
        text: `${texts[i % texts.length]} (#${i})`,
        timestamp: new Date(Date.now() - i * 30_000).toISOString(),
      }),
    });
  }
  return ops;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  const host = process.env.TOPGUN_HOST ?? 'server';
  const port = process.env.TOPGUN_PORT ?? '8080';
  const url = `ws://${host}:${port}/ws`;

  console.log(`Connecting to ${url}...`);

  let ws: WebSocket;
  try {
    ws = await connect(url);
  } catch (err) {
    console.error(`Failed to connect to ${url}:`, err);
    process.exit(1);
  }

  try {
    await authenticate(ws);

    // Seed tasks
    process.stdout.write('Seeding tasks... ');
    const taskOps = makeTasks('tasks');
    await seedMap(ws, 'tasks', taskOps);
    console.log(`${taskOps.length}/${taskOps.length} done`);

    // Seed users
    process.stdout.write('Seeding users... ');
    const userOps = makeUsers('users');
    await seedMap(ws, 'users', userOps);
    console.log(`${userOps.length}/${userOps.length} done`);

    // Seed messages
    process.stdout.write('Seeding messages... ');
    const msgOps = makeMessages('messages');
    await seedMap(ws, 'messages', msgOps);
    console.log(`${msgOps.length}/${msgOps.length} done`);

    const total = taskOps.length + userOps.length + msgOps.length;
    console.log(`Seeding complete: ${total} records across 3 maps`);

    ws.close();
    process.exit(0);
  } catch (err) {
    console.error('Seed failed:', err);
    ws.close();
    process.exit(1);
  }
}

main();
