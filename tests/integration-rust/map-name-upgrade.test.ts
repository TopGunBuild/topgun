/**
 * Integration: upgrading the server binary recovers writes the older binary
 * acknowledged but could not store.
 *
 * The older binary's embedded store accepts only identifier-shaped map names.
 * A write to `user-profiles` or `users/profiles` is acknowledged, its frame
 * stays in the WAL, and the store refuses it at every flush. This test makes
 * the older binary leave such frames behind, then starts the newer binary on
 * the same directory and checks that recovery replays them in order: the last
 * acknowledged state of every key is there, the removed key is gone, nothing
 * is reported as unreplayed, and the partitions' applied watermarks advance.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (OLD_SERVER_BINARY / OLD_SERVER_COMMIT for the older one,
 * RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles on its own
 * initiative.
 *
 * The watermark is checked through files, without decoding WAL frames: every
 * partition that had frames and no applied-watermark sidecar gets a sidecar,
 * and a second start of the newer binary finds nothing left to replay.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  requireTwoBinaryEnv,
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  merkleKey,
  observe,
  sleep,
  walPartitions,
  plainLines,
  SpawnedBinary,
  BinaryExit,
  TwoBinaryEnv,
} from './helpers/two-binary';

const CONTROL_MAP = 'user_profiles';
const REFUSED_BY_OLD = ['user-profiles', 'users/profiles'];

const VALUE_K1 = { name: 'removed later' };
const VALUE_K2 = { name: 'kept' };
const VALUE_CONTROL = { name: 'control' };

const ACK_WAIT_MS = 5_000;
/** The older binary holds a write for 1 s and then tries the store three times with backoff. */
const FLUSH_SETTLE_MS = 10_000;
/** A graceful stop may spend up to 30 s draining buffered writes; past this the process is killed and the exit status says so. */
const STOP_TIMEOUT_MS = 40_000;
const SIDECAR_WAIT_MS = 15_000;

interface MapState {
  queryK1: unknown;
  queryK2: unknown;
  merkleK1: unknown;
  merkleK2: unknown;
}

interface Observed {
  /** Per write, in the order sent: the types of the frames it was answered with. */
  writes: Array<{ op: string; answer: string[] | string }>;
  oldStop?: BinaryExit | null;
  oldAnsweredAfterSettle?: unknown;
  /** Partitions that held frames and had no sidecar when the older binary was gone. */
  pendingPartitions: number[];
  newPort?: number | null;
  firstBoot: Record<string, MapState>;
  controlFirstBoot?: unknown;
  firstBootUnreplayed?: string[];
  partitionsStillWithoutSidecar?: number[];
  newStop?: BinaryExit | null;
  secondPort?: number | null;
  secondBoot: Record<string, MapState>;
  controlSecondBoot?: unknown;
  secondBootUnreplayed?: string[];
  fatal: string[];
}

/** A removed key shows up as no row or as a null value, depending on the read path. */
function absent(value: unknown): boolean {
  return value === undefined || value === null;
}

describe('Integration: the newer server binary replays the WAL frames the older one could not store', () => {
  const o: Observed = {
    writes: [],
    pendingPartitions: [],
    firstBoot: {},
    secondBoot: {},
    fatal: [],
  };
  let dataDir: string | null = null;
  const running: SpawnedBinary[] = [];

  function start(binaryPath: string, commit: string, label: string): Promise<SpawnedBinary> {
    const dir = dataDir as string;
    return spawnBinary({
      binaryPath,
      commit,
      label,
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
        TOPGUN_WAL_FSYNC_POLICY: 'per_op',
      },
    }).then((server) => {
      running.push(server);
      return server;
    });
  }

  function unreplayedLines(server: SpawnedBinary): string[] {
    return plainLines(server.stdout())
      .filter((l) => l.includes('BootUnreplayed') || l.includes('replay failed for entry'))
      .map((l) => l.slice(0, 400));
  }

  function noteFatal(server: SpawnedBinary): void {
    for (const line of [...plainLines(server.stdout()), ...plainLines(server.stderr())]) {
      if (line.includes('FATAL')) o.fatal.push(`[${server.label}] ${line}`);
    }
  }

  async function readMap(port: number, mapName: string, tag: string): Promise<MapState> {
    return {
      queryK1: await observe(() => queryKey(port, mapName, 'k1', `upgrade-q1-${tag}`)),
      queryK2: await observe(() => queryKey(port, mapName, 'k2', `upgrade-q2-${tag}`)),
      merkleK1: await observe(() => merkleKey(port, mapName, 'k1', `upgrade-m1-${tag}`)),
      merkleK2: await observe(() => merkleKey(port, mapName, 'k2', `upgrade-m2-${tag}`)),
    };
  }

  beforeAll(async () => {
    const env: TwoBinaryEnv = requireTwoBinaryEnv();
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-map-name-upgrade-'));
    const walDir = path.join(dataDir, 'wal');

    // --- The older binary acknowledges the writes and leaves their frames behind.
    const old = await start(env.oldBinary, env.oldCommit, 'OLD');
    if (old.port === null) throw new Error(`OLD did not print PORT=; stderr:\n${old.stderr()}`);
    const oldPort = old.port;

    const writer = await connectWithin(oldPort, 'upgrade-writer');
    const millis = Date.now();
    let opId = 0;
    const write = async (
      mapName: string,
      opType: 'PUT' | 'REMOVE',
      key: string,
      value: unknown,
    ) => {
      opId += 1;
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: String(opId),
          mapName,
          opType,
          key,
          // Strictly increasing timestamps, so the removal is later than the write it removes.
          record: { value, timestamp: { millis: millis + opId, counter: 0, nodeId: 'upgrade' } },
        },
      });
      await firstFrame(writer, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
      o.writes.push({
        op: `${opType} ${mapName}/${key}`,
        answer: writer.messages.map((m) => m.type),
      });
    };
    for (const mapName of REFUSED_BY_OLD) {
      await write(mapName, 'PUT', 'k1', VALUE_K1);
      await write(mapName, 'PUT', 'k2', VALUE_K2);
      await write(mapName, 'REMOVE', 'k1', null);
    }
    await write(CONTROL_MAP, 'PUT', 'k', VALUE_CONTROL);
    writer.close();

    await sleep(FLUSH_SETTLE_MS);
    // Recorded, not asserted: whether the older binary still answers once its
    // flush attempts for the refused names have run.
    o.oldAnsweredAfterSettle = await observe(() =>
      queryKey(oldPort, CONTROL_MAP, 'k', 'upgrade-old-liveness'),
    );
    // The property under test needs the frames in the older binary's WAL with
    // no applied watermark; how that process ends does not change them, since
    // every acknowledged frame is already fsynced. The stop is asked for
    // gracefully and falls back to a kill, and the exit status is recorded.
    o.oldStop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);
    noteFatal(old);

    o.pendingPartitions = [...walPartitions(walDir).entries()]
      .filter(([, state]) => state.segmentBytes > 0 && !state.hasSidecar)
      .map(([id]) => id);

    // --- The newer binary boots on the same directory.
    const first = await start(env.newBinary, env.newCommit, 'NEW first run');
    o.newPort = first.port;
    if (first.port !== null) {
      for (const mapName of REFUSED_BY_OLD) {
        o.firstBoot[mapName] = await readMap(first.port, mapName, `first-${mapName.length}`);
      }
      const port = first.port;
      o.controlFirstBoot = await observe(() => queryKey(port, CONTROL_MAP, 'k', 'upgrade-c1'));

      const deadline = Date.now() + SIDECAR_WAIT_MS;
      const missing = () => {
        const now = walPartitions(walDir);
        return o.pendingPartitions.filter((id) => !now.get(id)?.hasSidecar);
      };
      while (missing().length > 0 && Date.now() < deadline) await sleep(250);
      o.partitionsStillWithoutSidecar = missing();
    }
    o.firstBootUnreplayed = unreplayedLines(first);
    o.newStop = await first.stop('SIGTERM', STOP_TIMEOUT_MS);
    noteFatal(first);

    // --- A second start of the newer binary finds nothing left to replay.
    const second = await start(env.newBinary, env.newCommit, 'NEW second run');
    o.secondPort = second.port;
    if (second.port !== null) {
      for (const mapName of REFUSED_BY_OLD) {
        o.secondBoot[mapName] = await readMap(second.port, mapName, `second-${mapName.length}`);
      }
      const port = second.port;
      o.controlSecondBoot = await observe(() => queryKey(port, CONTROL_MAP, 'k', 'upgrade-c2'));
    }
    o.secondBootUnreplayed = unreplayedLines(second);
    await second.stop('SIGTERM', STOP_TIMEOUT_MS);
    noteFatal(second);

    // Straight to stderr: the suite silences console.log.
    process.stderr.write(
      `[map-name-upgrade] observed:\n${JSON.stringify(o, null, 2)}\n` +
        running
          .map(
            (s) =>
              `[map-name-upgrade] ${s.label} WARN/ERROR lines:\n${plainLines(s.stdout())
                .filter((l) => /\b(WARN|ERROR)\b/.test(l))
                .map((l) => l.slice(0, 400))
                .join('\n')}\n[map-name-upgrade] ${s.label} stderr:\n${s.stderr()}\n`,
          )
          .join(''),
    );
  }, 420_000);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  test('setup: the older binary acknowledged every write', () => {
    expect(o.writes).toHaveLength(7);
    expect(o.writes.filter((w) => JSON.stringify(w.answer) !== '["OP_ACK"]')).toEqual([]);
  });

  test('setup: the older binary left WAL frames with no applied watermark', () => {
    expect(o.pendingPartitions.length).toBeGreaterThanOrEqual(1);
  });

  test('the newer binary starts on the directory, and no binary reports a fatal error', () => {
    expect(typeof o.newPort).toBe('number');
    expect(o.fatal).toEqual([]);
  });

  describe.each(REFUSED_BY_OLD)('map %p after the upgrade', (mapName) => {
    test('a query returns the kept value and not the removed key', () => {
      const state = o.firstBoot[mapName];
      expect(state?.queryK2).toEqual(VALUE_K2);
      expect(absent(state?.queryK1)).toBe(true);
    });

    test('a Merkle sync returns the kept value and not the removed key', () => {
      const state = o.firstBoot[mapName];
      expect(state?.merkleK2).toEqual(VALUE_K2);
      expect(absent(state?.merkleK1)).toBe(true);
    });

    test('the answers are the same after the newer binary is restarted', () => {
      expect(o.secondBoot[mapName]).toEqual(o.firstBoot[mapName]);
      expect(o.secondBoot[mapName]?.queryK2).toEqual(VALUE_K2);
    });
  });

  test('the control map is intact', () => {
    expect(o.controlFirstBoot).toEqual(VALUE_CONTROL);
    expect(o.controlSecondBoot).toEqual(VALUE_CONTROL);
  });

  test('the first boot of the newer binary reports nothing as unreplayed', () => {
    expect(o.firstBootUnreplayed).toEqual([]);
  });

  test('every partition that held un-applied frames now has an applied watermark', () => {
    expect(o.partitionsStillWithoutSidecar).toEqual([]);
  });

  test('the second boot of the newer binary has nothing left to replay', () => {
    expect(typeof o.secondPort).toBe('number');
    expect(o.secondBootUnreplayed).toEqual([]);
  });
});
