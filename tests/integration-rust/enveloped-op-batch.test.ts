/**
 * Integration: an operation batch carried inside a `BATCH` envelope is answered
 * per operation — a rejection for the refused write, an acknowledgement naming
 * the accepted ones — against the real Rust server.
 *
 * The batch holds three writes that all carry ids; the middle one names a map
 * the server cannot store. The two frames expected for it are written out
 * below: one rejection for the refused write and one acknowledgement naming the
 * two accepted ones. This test sends the batch only inside an envelope. It does
 * not send it on its own and compare; that comparison of the two paths is made
 * in process, by the Rust tests of the WebSocket handler.
 *
 * HOW THE ANSWER IS DELIMITED. The envelope holds a second item behind the
 * batch: a single write, the barrier. The server handles the items of one
 * envelope one after another, each to completion, and every frame of a
 * connection leaves through one first-in-first-out queue. So when the barrier's
 * acknowledgement arrives, every frame the batch was going to be answered with
 * has already arrived, and "the frames ahead of the barrier's acknowledgement"
 * is the whole answer — including the case where it is empty. Nothing here is
 * ordered by a delay: the only wait is for the barrier's acknowledgement, and
 * its deadline only turns a hang into a failure.
 *
 * A second envelope, or the same batch sent on its own on the same socket,
 * would delimit nothing: the server handles each inbound frame in a task of its
 * own, so answers to two frames of one connection are not ordered.
 *
 * The connection subscribes to nothing — no query, no topic, no journal — so
 * no frame reaches it except the answers to what it sent.
 *
 * Operation ids are numeric strings because the acknowledgement of a partly
 * refused batch is addressed by the highest numeric id that was accepted.
 */

import { deserialize, serialize } from '@topgunbuild/core';

import {
  spawnRustServer,
  createRustTestClient,
  createLWWRecord,
  SpawnedServer,
  TestClient,
} from './helpers';

/** Upper bound on any single wait. It only ever turns a hang into a failure. */
const SIGNAL_TIMEOUT_MS = 20_000;

const NODE_ID = 'enveloped-op-batch';
/** An identifier-shaped name every backend stores. */
const VALID_MAP = 'enveloped_op_batch';
/** The reserved backup suffix makes a name the server refuses where the write comes in. */
const REFUSED_MAP = 'enveloped_op_batch__backup';

const ACCEPTED_FIRST_ID = '101';
const REFUSED_ID = '102';
const ACCEPTED_LAST_ID = '103';
const BARRIER_ID = 'enveloped-op-batch-barrier';

interface InboundFrame {
  type?: string;
  payload?: {
    lastId?: string;
    opId?: string;
    results?: Array<{ opId?: string }> | null;
  };
}

/** One received frame as the pair the assertion compares: its type and the operation ids it names. */
type FrameSummary = [type: string, ids: string | string[] | null];

/** 4-byte big-endian length prefix, as the server's batch unpacking expects. */
function lenPrefix(n: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(n, 0);
  return b;
}

/** Packs inner messages as the envelope's `data` blob: each one behind its length prefix. */
function packBatchData(inner: Uint8Array[]): Buffer {
  return Buffer.concat(inner.flatMap((item) => [lenPrefix(item.length), Buffer.from(item)]));
}

/**
 * Hand-encodes the envelope `fixmap{ type:"BATCH", count:<n>, data:<bin> }`.
 *
 * The client `serialize()` walks a binary `data` field as if its bytes were
 * object keys, so the envelope is encoded directly to keep `data` a MsgPack
 * `bin` blob, which is what the server's batch body expects.
 */
function encodeBatchFrame(count: number, data: Uint8Array): Uint8Array {
  if (count < 0 || count > 0x7f) {
    throw new Error(`item count ${count} does not fit a MsgPack positive fixint`);
  }
  const head = Buffer.concat([
    Buffer.from([0x83]), // fixmap, 3 entries
    Buffer.from([0xa4]),
    Buffer.from('type'),
    Buffer.from([0xa5]),
    Buffer.from('BATCH'),
    Buffer.from([0xa5]),
    Buffer.from('count'),
    Buffer.from([count]),
    Buffer.from([0xa4]),
    Buffer.from('data'),
    Buffer.from([0xc6]), // bin32 marker; 4-byte big-endian length follows
  ]);
  return new Uint8Array(Buffer.concat([head, lenPrefix(data.length), Buffer.from(data)]));
}

function put(id: string, mapName: string, key: string): Record<string, unknown> {
  return {
    id,
    mapName,
    opType: 'PUT',
    key,
    record: createLWWRecord({ writtenBy: id }, NODE_ID),
  };
}

function summarize(frame: InboundFrame): FrameSummary {
  const type = String(frame.type);
  if (type === 'OP_REJECTED') return [type, frame.payload?.opId ?? null];
  if (type === 'OP_ACK') {
    const results = frame.payload?.results;
    // The accepted set, not the order the server happened to list it in.
    if (results) return [type, results.map((result) => String(result.opId)).sort()];
    return [type, frame.payload?.lastId ?? null];
  }
  return [type, null];
}

describe('Integration: an operation batch inside a BATCH envelope', () => {
  let server: SpawnedServer | null = null;
  let client: TestClient | null = null;
  /** Whether the barrier write was acknowledged within the deadline. */
  let barrierAcknowledged = false;
  /** Every frame received after the envelope was sent and before the barrier's acknowledgement. */
  let aheadOfBarrier: FrameSummary[] = [];

  beforeAll(async () => {
    server = await spawnRustServer();
    client = await createRustTestClient(server.port, {
      nodeId: NODE_ID,
      userId: NODE_ID,
      roles: ['ADMIN'],
    });

    // Attached in the same turn the socket opened in, so no frame is missed.
    const received: InboundFrame[] = [];
    let onBarrier: (() => void) | null = null;
    client.ws.on('message', (data: ArrayBuffer | Buffer) => {
      const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
      const frame = deserialize(bytes as Uint8Array) as InboundFrame;
      received.push(frame);
      if (frame.type === 'OP_ACK' && frame.payload?.lastId === BARRIER_ID) onBarrier?.();
    });
    await client.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);

    const barrier = new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => resolve(false), SIGNAL_TIMEOUT_MS);
      onBarrier = () => {
        clearTimeout(timer);
        resolve(true);
      };
    });

    const batch = serialize({
      type: 'OP_BATCH',
      payload: {
        ops: [
          put(ACCEPTED_FIRST_ID, VALID_MAP, 'key-first'),
          put(REFUSED_ID, REFUSED_MAP, 'key-refused'),
          put(ACCEPTED_LAST_ID, VALID_MAP, 'key-last'),
        ],
      },
    });
    const barrierWrite = serialize({
      type: 'CLIENT_OP',
      payload: put(BARRIER_ID, VALID_MAP, 'key-barrier'),
    });
    // Everything from here on is the answer to this one frame: the handshake
    // is complete and the connection has asked for nothing else.
    const sentAt = received.length;
    client.ws.send(encodeBatchFrame(2, packBatchData([batch, barrierWrite])));

    barrierAcknowledged = await barrier;
    const barrierAt = received.findIndex(
      (frame, index) =>
        index >= sentAt && frame.type === 'OP_ACK' && frame.payload?.lastId === BARRIER_ID,
    );
    aheadOfBarrier = received
      .slice(sentAt, barrierAt === -1 ? received.length : barrierAt)
      .map(summarize);
  }, 180_000);

  afterAll(async () => {
    if (client) client.close();
    if (server) await server.cleanup().catch(() => {});
  });

  test('is answered with one rejection and one acknowledgement of the accepted writes', () => {
    // Without the barrier's acknowledgement the frames collected say nothing
    // about the batch: the envelope may not have been served at all.
    expect({ barrierAcknowledged }).toEqual({ barrierAcknowledged: true });

    expect(aheadOfBarrier).toEqual([
      ['OP_REJECTED', REFUSED_ID],
      ['OP_ACK', [ACCEPTED_FIRST_ID, ACCEPTED_LAST_ID]],
    ]);
  });
});
