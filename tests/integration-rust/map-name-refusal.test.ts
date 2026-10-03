/**
 * Integration: a map name the server cannot store is refused where the write
 * comes in — before it is logged, before it is applied, before it is
 * acknowledged — and each entry path reports that in its own way.
 *
 * Four names are refused: one ending in the reserved `__backup` suffix, the
 * empty name, a name longer than 512 bytes, and a name holding U+0000. For
 * each of them, on the embedded store and on the in-memory one:
 *
 * - in a batch whose operations all carry ids, the refused operation gets its
 *   own rejection (code 400) and the valid ones around it are applied and
 *   acknowledged;
 * - in a batch where an operation has no id, the refusal cannot be attributed,
 *   so the batch gets one error (code 400), nothing of it is applied and
 *   nothing is acknowledged;
 * - a single write, a query subscription and a sync request get no frame at
 *   all — the operator's log line is the signal there;
 * - over HTTP the refusal is one entry of the response's error list;
 * - on the embedded store, the write-ahead log never holds the refused
 *   operation.
 *
 * Every name gets a server process of its own: the operator's log line is
 * written at most once a minute per process, so a shared server would log the
 * first name only.
 *
 * Operation ids are numeric strings because the acknowledgement of a partly
 * refused batch is addressed by the highest numeric id that was accepted;
 * with other ids there is no acknowledgement to observe.
 */

import * as crypto from 'crypto';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { serialize, deserialize } from '@topgunbuild/core';

import { createLWWRecord, TestClient } from './helpers';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  observe,
  sleep,
  countBytesUnder,
  plainLines,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

const BACKENDS = ['redb', 'null'] as const;

const REFUSED = [
  { label: 'x__backup (reserved suffix)', name: 'x__backup', violation: 'ReservedBackupSuffix' },
  { label: 'the empty name', name: '', violation: 'Empty' },
  { label: 'a 513-byte name', name: 'n'.repeat(513), violation: 'TooLong' },
  { label: 'a name holding U+0000', name: 'a\u0000b', violation: 'ContainsNul' },
];

/** An identifier-shaped name every backend stores, for the valid operations. */
const VALID_MAP = 'refusal_valid';
const SILENCE_MS = 3_000;
const FRAME_WAIT_MS = 3_000;
/** The operator's log line is written at most once per 60 s; past this the "exactly one line" clause proves nothing. */
const LIMITER_WINDOW_GUARD_MS = 55_000;
/** A graceful stop may spend up to 30 s draining buffered writes; past this the process is killed and the exit status says so. */
const STOP_TIMEOUT_MS = 40_000;

function serverBinary(): string {
  const binary =
    process.env.RUST_SERVER_BINARY ??
    path.resolve(__dirname, '..', '..', 'target', 'release', 'topgun-server');
  if (!fs.existsSync(binary)) {
    throw new Error(
      `No server binary at ${binary}. Build it (cargo build --release -p topgun-server) or set RUST_SERVER_BINARY.`,
    );
  }
  return binary;
}

interface Frames {
  acks: any[];
  rejected: any[];
  errors: any[];
  all: string[];
}

/** Unwraps an observation, failing with the reason when the probe itself could not be made. */
function seen<T>(value: T | string | undefined): T {
  if (value === undefined || typeof value === 'string') {
    throw new Error(String(value ?? 'NOT OBSERVED: the step did not run'));
  }
  return value;
}

function sortFrames(messages: any[]): Frames {
  return {
    acks: messages.filter((m) => m.type === 'OP_ACK').map((m) => m.payload),
    rejected: messages.filter((m) => m.type === 'OP_REJECTED').map((m) => m.payload),
    errors: messages.filter((m) => m.type === 'ERROR').map((m) => m.payload),
    all: messages.map((m) => m.type),
  };
}

interface Observed {
  /** A batch of [valid, refused, valid], every operation carrying an id. */
  batch?: Frames | string;
  batchValueA?: unknown;
  batchValueC?: unknown;
  /** Occurrences of the markers in the WAL right after that batch was answered. */
  walAfterBatch?: { refused: number; validA: number; validC: number; files: string[] } | string;
  /** A batch of [valid without id, refused with id] on one key. */
  idless?: Frames | string;
  idlessValue?: unknown;
  /** Types of every frame received within the silence window, per single-message path. */
  clientOp?: string[] | string;
  querySub?: string[] | string;
  syncInit?: string[] | string;
  http?: { status: number; errors: any[] | undefined; ack: any } | string;
  /** The operator's log lines for refused names, from the server's stdout. */
  refusalLogLines?: string[];
  msFromFirstRefusalToLogRead?: number;
  /** Occurrences of each refused marker in the WAL just before the server is stopped. */
  walBeforeStop?: Record<string, number> | string;
  /** The in-memory backend is expected to write no WAL; recorded, and searched like the other if it does. */
  walDirExists?: boolean;
  stop?: BinaryExit | null;
  restartPort?: number | null;
  restartBootUnreplayed?: string[];
  fatal?: string[];
}

describe.each(BACKENDS)(
  'Integration: a map name outside the storable set is refused (%s)',
  (backend) => {
    describe.each(REFUSED)('$label', ({ name, violation }) => {
      const o: Observed = {};
      const markers = {
        validA: `VALID-${crypto.randomUUID()}`,
        validC: `VALID-${crypto.randomUUID()}`,
        refused: [] as string[],
      };
      let dataDir: string | null = null;
      let server: SpawnedBinary | null = null;

      function refusedValue(): { marker: string } {
        const marker = `REFUSED-${crypto.randomUUID()}`;
        markers.refused.push(marker);
        return { marker };
      }

      function put(
        mapName: string,
        key: string,
        value: unknown,
        id?: string,
      ): Record<string, unknown> {
        const op: Record<string, unknown> = {
          mapName,
          opType: 'PUT',
          key,
          record: createLWWRecord(value),
        };
        if (id !== undefined) op.id = id;
        return op;
      }

      function start(dir: string, label: string): Promise<SpawnedBinary> {
        const env: Record<string, string> = { STORAGE_BACKEND: backend };
        if (backend === 'redb') {
          env.TOPGUN_REDB_PATH = path.join(dir, 'topgun.redb');
          env.TOPGUN_WAL_DIR = path.join(dir, 'wal');
          env.TOPGUN_WAL_FSYNC_POLICY = 'per_op';
        }
        return spawnBinary({ binaryPath: serverBinary(), label, env });
      }

      /** Sends one message on a fresh connection and returns the types of every frame that came back in the window. */
      async function framesAfter(
        port: number,
        nodeId: string,
        message: Record<string, unknown>,
      ): Promise<string[]> {
        const client = await connectWithin(port, nodeId);
        try {
          client.messages.length = 0;
          client.send(message);
          await sleep(SILENCE_MS);
          return client.messages.map((m) => m.type);
        } finally {
          client.close();
        }
      }

      async function batchRound(
        client: TestClient,
        ops: Record<string, unknown>[],
        settleOn: string[],
      ): Promise<Frames> {
        client.messages.length = 0;
        client.send({ type: 'OP_BATCH', payload: { ops } });
        await firstFrame(client, settleOn, FRAME_WAIT_MS);
        // Whatever else the exchange sends follows within the same turn of the
        // server's writer; a short pause lets a second frame land.
        await sleep(300);
        return sortFrames(client.messages);
      }

      beforeAll(async () => {
        dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-refusal-'));
        const walDir = path.join(dataDir, 'wal');
        server = await start(dataDir, `refusal ${backend} ${violation}`);
        if (server.port === null) {
          throw new Error(`server did not print PORT=; stderr:\n${server.stderr()}`);
        }
        const port = server.port;
        const tag = `${backend}-${violation}`;
        let firstRefusalAt = 0;

        o.batch = await observe(async () => {
          const client = await connectWithin(port, `refusal-batch-${tag}`);
          try {
            firstRefusalAt = Date.now();
            return await batchRound(
              client,
              [
                put(VALID_MAP, 'a', { marker: markers.validA }, '1'),
                put(name, 'b', refusedValue(), '2'),
                put(VALID_MAP, 'c', { marker: markers.validC }, '3'),
              ],
              ['OP_ACK'],
            );
          } finally {
            client.close();
          }
        });
        if (firstRefusalAt === 0) firstRefusalAt = Date.now();

        if (backend === 'redb') {
          o.walAfterBatch = await observe(async () => ({
            refused: countBytesUnder(walDir, markers.refused[0]),
            validA: countBytesUnder(walDir, markers.validA),
            validC: countBytesUnder(walDir, markers.validC),
            files: fs.existsSync(walDir) ? fs.readdirSync(walDir) : [],
          }));
        }
        o.batchValueA = await observe(() => queryKey(port, VALID_MAP, 'a', `refusal-qa-${tag}`));
        o.batchValueC = await observe(() => queryKey(port, VALID_MAP, 'c', `refusal-qc-${tag}`));

        if (name === 'x__backup') {
          o.http = await observe(async () => {
            const body = serialize({
              clientId: 'refusal-http',
              clientHlc: { millis: Date.now(), counter: 0, nodeId: 'refusal-http' },
              operations: [
                put(VALID_MAP, 'http', { via: 'http' }, '11'),
                put(name, 'http', refusedValue(), '12'),
              ],
            });
            const response = await fetch(`http://127.0.0.1:${port}/sync`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/msgpack' },
              body: Buffer.from(body),
              signal: AbortSignal.timeout(5_000),
            });
            const decoded = deserialize<any>(new Uint8Array(await response.arrayBuffer()));
            return { status: response.status, errors: decoded.errors, ack: decoded.ack };
          });
        }

        o.idless = await observe(async () => {
          const client = await connectWithin(port, `refusal-idless-${tag}`);
          try {
            // Both operations use one key, so they fall in one partition and the
            // server handles them as one sub-batch.
            return await batchRound(
              client,
              [
                put(VALID_MAP, 'shared', { applied: 'must not be' }),
                put(name, 'shared', refusedValue(), '5'),
              ],
              ['ERROR', 'OP_ACK', 'OP_REJECTED'],
            );
          } finally {
            client.close();
          }
        });
        o.idlessValue = await observe(() =>
          queryKey(port, VALID_MAP, 'shared', `refusal-qs-${tag}`),
        );

        // The three silent paths are watched at the same time, each on a socket
        // of its own, so the whole exchange with this process stays well inside
        // one window of the operator's log limiter.
        [o.clientOp, o.querySub, o.syncInit] = await Promise.all([
          observe(() =>
            framesAfter(port, `refusal-op-${tag}`, {
              type: 'CLIENT_OP',
              payload: put(name, 'single', refusedValue(), '7'),
            }),
          ),
          observe(() =>
            framesAfter(port, `refusal-query-${tag}`, {
              type: 'QUERY_SUB',
              payload: { queryId: `q-refused-${tag}`, mapName: name, query: {} },
            }),
          ),
          observe(() =>
            framesAfter(port, `refusal-sync-${tag}`, { type: 'SYNC_INIT', mapName: name }),
          ),
        ]);

        o.msFromFirstRefusalToLogRead = Date.now() - firstRefusalAt;
        o.refusalLogLines = plainLines(server.stdout()).filter(
          (line) =>
            /\bWARN\b/.test(line) && line.includes('map_name=') && line.includes('suppressed='),
        );
        o.walDirExists = fs.existsSync(walDir);
        if (o.walDirExists) {
          o.walBeforeStop = await observe(async () =>
            Object.fromEntries(markers.refused.map((m) => [m, countBytesUnder(walDir, m)])),
          );
        }

        o.stop = await server.stop('SIGTERM', STOP_TIMEOUT_MS);
        o.fatal = [...plainLines(server.stdout()), ...plainLines(server.stderr())].filter((line) =>
          line.includes('FATAL'),
        );
        const firstRunWarnings = plainLines(server.stdout()).filter((l) =>
          /\b(WARN|ERROR)\b/.test(l),
        );

        if (backend === 'redb') {
          server = await start(dataDir, `refusal ${backend} ${violation} restart`);
          o.restartPort = server.port;
          o.restartBootUnreplayed = plainLines(server.stdout()).filter(
            (line) => line.includes('BootUnreplayed') || line.includes('replay failed for entry'),
          );
          await server.stop('SIGTERM', STOP_TIMEOUT_MS);
        }
        server = null;

        // Straight to stderr (the suite silences console.log): what this process
        // did, kept whether or not an assertion fails.
        process.stderr.write(
          `[map-name-refusal] ${backend} ${JSON.stringify(name.length > 40 ? `${name.slice(0, 8)}… (${name.length} bytes)` : name)}:\n` +
            `${JSON.stringify(o, null, 2)}\n` +
            `first-run WARN/ERROR lines (${firstRunWarnings.length}):\n${firstRunWarnings
              .map((l) => l.slice(0, 400))
              .join('\n')}\n`,
        );
      }, 240_000);

      afterAll(async () => {
        if (server) await server.stop('SIGKILL', 5_000);
        if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
      });

      test('batch with ids: the refused operation gets one rejection with code 400', () => {
        const batch = seen(o.batch);
        expect(batch.rejected).toHaveLength(1);
        expect(batch.rejected[0]).toMatchObject({ opId: '2', code: 400, permanent: true });
        expect(batch.errors).toHaveLength(0);
      });

      test('batch with ids: both valid operations are acknowledged by id and applied', () => {
        const batch = seen(o.batch);
        expect(batch.acks).toHaveLength(1);
        const ackedIds = ((batch.acks[0].results ?? []) as any[]).map((r) => r.opId).sort();
        expect(ackedIds).toEqual(['1', '3']);
        expect(batch.acks[0].lastId).toBe('3');
        expect(o.batchValueA).toEqual({ marker: markers.validA });
        expect(o.batchValueC).toEqual({ marker: markers.validC });
      });

      test('batch with an id-less operation: one batch-level error with code 400, nothing acknowledged, nothing applied', () => {
        const idless = seen(o.idless);
        expect(idless.errors).toHaveLength(1);
        expect(idless.errors[0]).toMatchObject({ code: 400 });
        expect(idless.rejected).toHaveLength(0);
        expect(idless.acks).toHaveLength(0);
        expect(o.idlessValue).toBeUndefined();
      });

      test('a single write gets no frame of any type', () => {
        expect(o.clientOp).toEqual([]);
      });

      test('a query subscription gets no frame of any type', () => {
        expect(o.querySub).toEqual([]);
      });

      test('a sync request gets no frame of any type', () => {
        expect(o.syncInit).toEqual([]);
      });

      test('the server logs exactly one warning for the refusals, naming the violation', () => {
        if ((o.msFromFirstRefusalToLogRead ?? Infinity) >= LIMITER_WINDOW_GUARD_MS) {
          throw new Error(
            `run exceeded the limiter window: ${o.msFromFirstRefusalToLogRead} ms from the first refusal to the log read`,
          );
        }
        expect(o.refusalLogLines).toHaveLength(1);
        expect(o.refusalLogLines![0]).toContain(violation);
      });

      test('the server never reports a fatal error', () => {
        expect(o.fatal).toEqual([]);
      });

      if (name === 'x__backup') {
        test('HTTP sync: one error entry with code 400 for the refused operation; the valid one is acknowledged', () => {
          const http = seen(o.http);
          expect(http.status).toBe(200);
          expect(http.errors).toHaveLength(1);
          expect(http.errors![0]).toMatchObject({ code: 400, context: '12' });
          expect(http.ack?.lastId).toBe('11');
        });
      }

      if (backend === 'redb') {
        test('the WAL holds the valid operations of the batch (the search can see frame payloads)', () => {
          const wal = seen(o.walAfterBatch);
          expect(wal.validA).toBeGreaterThanOrEqual(1);
          expect(wal.validC).toBeGreaterThanOrEqual(1);
        });

        test('the WAL holds no frame for a refused operation', () => {
          const wal = seen(o.walAfterBatch);
          expect(wal.refused).toBe(0);
          const beforeStop = seen(o.walBeforeStop);
          expect(Object.keys(beforeStop)).toHaveLength(markers.refused.length);
          expect(Object.values(beforeStop).filter((count) => count !== 0)).toEqual([]);
        });

        test('after a clean stop, a restart on the same directory has nothing left to replay', () => {
          expect(o.stop).toEqual({ code: 0, signal: null });
          expect(typeof o.restartPort).toBe('number');
          expect(o.restartBootUnreplayed).toEqual([]);
        });
      }
    });
  },
);
