/**
 * Integration tests for the bounded-recursion MsgPack decode defense.
 *
 * Background — and an honesty note. A deeply-nested MsgPack frame drives one
 * native stack frame per nesting level inside a *naive* recursive decoder; an
 * unbounded such decode overflows the stack and aborts the WHOLE process
 * (uncatchable in safe Rust). HOWEVER, the pinned codec `rmp_serde` 1.3.1 already
 * caps its own recursion at 1024 levels and returns a graceful `DepthLimitExceeded`
 * rather than overflowing. So on the current dependency a hostile deep frame is
 * handled gracefully WITH OR WITHOUT our fix — a "the process survives" assertion
 * alone is therefore a property of the dependency, not proof of our change.
 *
 * The server's own defense (`decode_depth_checked`) is a *version-independent*,
 * strictly tighter bound (MAX_DECODE_DEPTH = 256, enforced by an iterative
 * pre-scan that itself never recurses), so it keeps holding if the codec is ever
 * swapped or its internal cap removed. This suite therefore has two kinds of test:
 *
 *  1. Behavioral coverage (graceful reject + liveness + negative controls) over
 *     all three untrusted-inbound paths: pre-auth `/ws` (Phase 1), `/sync`, and
 *     inner messages inside an authenticated OpBatch. These confirm the wire-level
 *     handling is graceful; they pass on the dependency alone.
 *
 *  2. The LOAD-BEARING, fix-specific DISCRIMINATOR (`inner message ... within
 *     rmp_serde's 1024 limit is dropped`): a *valid* inner op nested between our
 *     256 bound and the codec's 1024 ceiling. Our scanner drops it; raw rmp_serde
 *     would accept and process it. This test FAILS if the inner-batch decode is
 *     not routed through `decode_depth_checked` — i.e. it actually exercises the
 *     fix, not the dependency. (Its unit-level twin is
 *     `decode::tests::scanner_rejects_within_rmp_serde_own_limit`.)
 */

import WebSocket from 'ws';
import { serialize } from '@topgunbuild/core';
import { spawnRustServer, createRustTestClient, createLWWRecord, waitForSync } from './helpers';
import type { SpawnedServer } from './helpers';

// Deep enough that BOTH our 256 scanner bound AND rmp_serde's own 1024 ceiling
// reject it — used for the graceful-reject/liveness coverage. ~2 KB frame, far
// under the 2 MB inbound cap, so it reaches the decoder rather than the size gate.
const DEEP_NESTING = 2_000;

// In the (256, 1024] window: above OUR scanner bound but BELOW rmp_serde's own
// limit. This is what discriminates the fix from the dependency.
const MODERATE_NESTING = 500;

/**
 * Builds a `fixmap(1) { "k": <depth nested 1-element arrays> nil }` payload.
 *
 * The deep chain is wrapped as a MAP VALUE, not sent bare: a bare nested array is
 * rejected *shallowly* by the internally-tagged `TopGunMessage` decoder (which
 * needs a map to find the `type` tag) and by the `/sync` struct decoder, so it
 * never recurses. Burying the chain under a map key forces serde to traverse it.
 */
function deepFrame(depth = DEEP_NESTING): Buffer {
  const head = Buffer.from([0x81, 0xa1, 0x6b]); // fixmap(1), fixstr "k"
  return Buffer.concat([head, Buffer.alloc(depth, 0x91), Buffer.from([0xc0])]);
}

/** A `depth`-deep nested 1-element JS array — a valid `rmpv::Value` record value. */
function deepArray(depth: number): unknown {
  let a: unknown = 0;
  for (let i = 0; i < depth; i++) a = [a];
  return a;
}

/**
 * A structurally VALID CLIENT_OP whose `record.value` nests `depth` deep. Decodes
 * fine through raw rmp_serde (when depth < 1024) and dispatches to an OP_ACK — so
 * it is the right probe for whether the inner-batch site applies our tighter bound.
 */
function validDeepClientOp(id: string, depth: number): Uint8Array {
  return serialize({
    type: 'CLIENT_OP',
    payload: {
      id,
      mapName: 'decode-dos-disc',
      opType: 'PUT',
      key: 'dk',
      record: {
        value: deepArray(depth),
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'decode-dos-node' },
      },
    },
  });
}

/** 4-byte big-endian length prefix, as `unpack_and_dispatch_batch` expects. */
function lenPrefix(n: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(n, 0);
  return b;
}

/** Wraps inner-message bytes as a single-entry OpBatch `data` blob. */
function packBatchData(inner: Uint8Array): Buffer {
  return Buffer.concat([lenPrefix(inner.length), Buffer.from(inner)]);
}

/**
 * Hand-encodes the outer OpBatch frame: `fixmap{ type:"BATCH", count:1, data:<bin> }`.
 *
 * The client `serialize()` runs `stripUndefined`, which walks a binary `data`
 * field as if its bytes were object keys and corrupts it into a huge map. The
 * envelope is therefore encoded directly so `data` stays a MsgPack `bin` blob —
 * exactly what the server's `serde_bytes` batch body expects, and what lets the
 * outer depth scan skip it (the whole point of the inner-batch attack surface).
 */
function encodeBatchFrame(data: Uint8Array): Uint8Array {
  const head = Buffer.from([
    0x83, // fixmap, 3 entries
    0xa4,
    0x74,
    0x79,
    0x70,
    0x65, // "type"
    0xa5,
    0x42,
    0x41,
    0x54,
    0x43,
    0x48, // "BATCH"
    0xa5,
    0x63,
    0x6f,
    0x75,
    0x6e,
    0x74, // "count"
    0x01, // 1
    0xa4,
    0x64,
    0x61,
    0x74,
    0x61, // "data"
    0xc6, // bin32 marker; 4-byte big-endian length follows
  ]);
  return new Uint8Array(Buffer.concat([head, lenPrefix(data.length), Buffer.from(data)]));
}

/**
 * Returns a predicate that reports whether the spawned server process has died.
 * A liveness sanity check — with rmp_serde's 1024 cap a crash is not expected, but
 * if a regression ever did abort the process this trips directly.
 */
function exitTracker(server: SpawnedServer): () => boolean {
  let exited = false;
  server.process.once('exit', () => {
    exited = true;
  });
  return () => exited || server.process.exitCode !== null || server.process.signalCode !== null;
}

type InboundMessage = { type: string; payload?: { lastId?: string; code?: number } };

/** True if an OP_ACK referencing `lastId` has been received. */
function sawOpAck(messages: InboundMessage[], lastId: string): boolean {
  return messages.some((m) => m.type === 'OP_ACK' && m.payload?.lastId === lastId);
}

/** True if an ERROR frame carrying `code` has been received. */
function sawError(messages: InboundMessage[], code: number): boolean {
  return messages.some((m) => m.type === 'ERROR' && m.payload?.code === code);
}

/** Polls until an ERROR frame with `code` arrives; returns whether it did. */
async function waitForError(
  messages: InboundMessage[],
  code: number,
  timeout = 5_000,
): Promise<boolean> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (sawError(messages, code)) return true;
    await waitForSync(50);
  }
  return sawError(messages, code);
}

/** Polls until an OP_ACK for `lastId` arrives, or throws on timeout. */
async function waitForOpAck(
  messages: InboundMessage[],
  lastId: string,
  timeout = 10_000,
): Promise<void> {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (sawOpAck(messages, lastId)) return;
    await waitForSync(50);
  }
  throw new Error(`timeout waiting for OP_ACK lastId=${lastId}`);
}

describe('Integration: MsgPack decode depth defense (Rust Server)', () => {
  let server: SpawnedServer;
  let port: number;
  let hasExited: () => boolean;

  beforeAll(async () => {
    server = await spawnRustServer();
    port = server.port;
    hasExited = exitTracker(server);
  });

  afterAll(async () => {
    await server.cleanup();
  });

  /** Authenticates a fresh client and round-trips a shallow PUT → OP_ACK. */
  async function assertServerStillServing(label: string): Promise<void> {
    const probe = await createRustTestClient(port);
    await probe.waitForMessage('AUTH_ACK', 10_000);
    probe.messages.length = 0;
    const id = `survive-${label}-${Date.now()}`;
    probe.send({
      type: 'CLIENT_OP',
      payload: {
        id,
        mapName: 'decode-dos-survive',
        opType: 'PUT',
        key: 'k',
        record: createLWWRecord({ ok: true }),
      },
    });
    const ack = await probe.waitForMessage('OP_ACK', 10_000);
    expect(ack.type).toBe('OP_ACK');
    probe.close();
  }

  describe('pre-auth /ws (Phase 1)', () => {
    test('deeply-nested frame is rejected gracefully and the server stays alive', async () => {
      const client = await createRustTestClient(port, { autoAuth: false });
      await client.waitForMessage('AUTH_REQUIRED', 10_000);

      // Hostile frame BEFORE authenticating — the Phase 1 pre-auth decode path.
      client.ws.send(deepFrame());
      await waitForSync(500);

      // The frame is dropped, not interpreted as an AUTH: no AUTH_ACK arrives.
      expect(client.isAuthenticated).toBe(false);
      expect(hasExited()).toBe(false);
      // A fresh connection still authenticates and round-trips.
      await assertServerStillServing('ws-preauth');

      client.close();
    });

    test('negative control: a shallow frame authenticates and processes normally', async () => {
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);
      expect(client.ws.readyState).toBe(WebSocket.OPEN);
      expect(client.isAuthenticated).toBe(true);
      client.close();
    });
  });

  describe('POST /sync', () => {
    test('deeply-nested body returns HTTP 400 and the server stays alive', async () => {
      const resp = await fetch(`http://127.0.0.1:${port}/sync`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/msgpack' },
        body: new Uint8Array(deepFrame()),
      });

      expect(resp.status).toBe(400);
      expect(hasExited()).toBe(false);

      // Negative control + liveness: a valid shallow /sync returns 200.
      const ok = await fetch(`http://127.0.0.1:${port}/sync`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/msgpack' },
        body: new Uint8Array(
          serialize({
            clientId: 'decode-dos-sync',
            clientHlc: { millis: Date.now(), counter: 0, nodeId: 'decode-dos-node' },
          }),
        ),
      });
      expect(ok.status).toBe(200);
    });
  });

  describe('inner message inside an OpBatch (post-auth)', () => {
    test('deeply-nested inner message is rejected gracefully and the server stays alive', async () => {
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);

      // OpBatch whose opaque `bin` data packs one deeply-nested inner message.
      // The outer frame is shallow, so it passes the outer scan and reaches
      // unpack_and_dispatch_batch, where the inner gets its own depth-checked decode.
      client.ws.send(encodeBatchFrame(packBatchData(deepFrame())));
      await waitForSync(500);

      expect(hasExited()).toBe(false);

      // Liveness: the same connection still round-trips a shallow op.
      client.messages.length = 0;
      const id = `batch-survive-${Date.now()}`;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id,
          mapName: 'decode-dos-batch',
          opType: 'PUT',
          key: 'k',
          record: createLWWRecord({ ok: true }),
        },
      });
      await waitForOpAck(client.messages, id);

      client.close();
    });

    test('DISCRIMINATOR: a valid inner op nested beyond our 256 bound but within rmp_serde 1024 is dropped (not processed)', async () => {
      // This is the fix-specific assertion. The inner op is structurally VALID and
      // nests ~500 deep — above MAX_DECODE_DEPTH (256) but below rmp_serde's 1024
      // limit. Patched (:754 → decode_depth_checked): the scanner drops it, so NO
      // OP_ACK for its id is ever produced. Unpatched (:754 → raw rmp_serde): the op
      // decodes (500 < 1024), dispatches, and acks → this test FAILS. That is what
      // makes it exercise the fix and not merely the dependency.
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);
      client.messages.length = 0;

      const deepId = `disc-deep-${Date.now()}`;
      const probeId = `disc-probe-${Date.now()}`;

      client.ws.send(encodeBatchFrame(packBatchData(validDeepClientOp(deepId, MODERATE_NESTING))));
      // A shallow probe op on the same in-order connection. Once it acks, the deep
      // inner has been processed-or-dropped, so the absence of its ack is decisive.
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: probeId,
          mapName: 'decode-dos-disc',
          opType: 'PUT',
          key: 'pk',
          record: createLWWRecord({ ok: true }),
        },
      });

      await waitForOpAck(client.messages, probeId);

      expect(sawOpAck(client.messages, deepId)).toBe(false);
      expect(hasExited()).toBe(false);

      client.close();
    });

    test('batch loop continues past a dropped over-deep inner item (multi-item batch)', async () => {
      // [over-deep inner (rejected), shallow valid inner] packed into ONE batch.
      // The over-deep first item must be dropped with `continue` — NOT break/return
      // — so the second item is still classified, dispatched, and acked. This fails
      // if the inner-decode error arm ever tears down the batch loop (silent loss of
      // every inner item after a hostile one).
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);
      client.messages.length = 0;

      const id = `batch-multi-${Date.now()}`;
      const deep = deepFrame(); // map-wrapped, rejected by the scanner
      const shallow = serialize({
        type: 'CLIENT_OP',
        payload: {
          id,
          mapName: 'decode-dos-multi',
          opType: 'PUT',
          key: 'k',
          record: createLWWRecord({ ok: true }),
        },
      });
      const data = Buffer.concat([
        lenPrefix(deep.length),
        deep,
        lenPrefix(shallow.length),
        Buffer.from(shallow),
      ]);
      client.ws.send(encodeBatchFrame(data));

      // The shallow second item is acked only if the loop continued past the drop.
      await waitForOpAck(client.messages, id);

      client.close();
    });

    test('negative control: a shallow inner message inside an OpBatch is processed', async () => {
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);
      client.messages.length = 0;

      const id = `batch-inner-${Date.now()}`;
      const inner = serialize({
        type: 'CLIENT_OP',
        payload: {
          id,
          mapName: 'decode-dos-batch-neg',
          opType: 'PUT',
          key: 'k',
          record: createLWWRecord({ ok: true }),
        },
      });
      client.ws.send(encodeBatchFrame(packBatchData(inner)));

      // The shallow inner op is classified, dispatched, and acked — proving the
      // drops above are depth-driven, not a blanket batch drop.
      await waitForOpAck(client.messages, id);

      client.close();
    });
  });

  describe('transport Batch rate-limiter charge follows actual inner-item count', () => {
    test('LOAD-BEARING: count=1 frames packing >40k framed items are charged their true count and trigger a 429', async () => {
      // Each outer frame declares count=1 (encodeBatchFrame hardcodes it) but packs
      // 40_001 well-formed zero-length framed inner items (each a 4-byte BE length of
      // 0). The server's count_batch_items returns 40_001 per frame. The cost is
      // clamped to the bucket capacity (40_000) at the charge site, so the FIRST such
      // frame drains the full burst to ~0 (and succeeds); the SECOND frame, arriving a
      // few ms later with only a sliver of refill available, cannot cover its clamped
      // cost → try_consume fails → a 429 ERROR fires at the CHARGE SITE, before any
      // inner item is decoded or dispatched (inner validity is irrelevant). ~160 KB
      // per frame, well under the 2 MB inbound cap.
      //
      // WHY this discriminates the fix: under the pre-fix model each frame costs the
      // declared count (1), so neither frame meaningfully drains the bucket and NO 429
      // is produced. Revert inbound_op_cost's Batch arm to `b.count as usize` and this
      // test goes red. (Burst is the hardcoded 40_000 in the spawned server — the
      // over-burst approach is required; it is not env-tunable.)
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);
      client.messages.length = 0;

      const ITEMS = 40_001;
      // ITEMS framed len-0 entries = ITEMS * 4 zero bytes (each 4-byte BE length = 0).
      const data = Buffer.alloc(ITEMS * 4, 0);
      // First frame drains the burst; second finds the bucket empty → 429.
      client.ws.send(encodeBatchFrame(data));
      client.ws.send(encodeBatchFrame(data));

      const got429 = await waitForError(client.messages, 429);
      expect(got429).toBe(true);
      expect(hasExited()).toBe(false);

      client.close();
    });

    test('negative control: a count=1 frame packing a few valid ops is charged its true small count and processed (no 429)', async () => {
      // A legitimate small batch: count=1 on the wire, ~5 valid shallow inner ops
      // packed in data. Its true charge is ~5, far under the burst, so it is admitted
      // and processed normally — no 429. Proves the >40k rejection above is the
      // amplification guard, not a blanket batch penalty.
      const client = await createRustTestClient(port);
      await client.waitForMessage('AUTH_ACK', 10_000);
      client.messages.length = 0;

      const ids = Array.from({ length: 5 }, (_, i) => `batch-charge-neg-${i}-${Date.now()}`);
      const data = Buffer.concat(
        ids.flatMap((id) => {
          const inner = validDeepClientOp(id, 1); // shallow, valid CLIENT_OP
          return [lenPrefix(inner.length), Buffer.from(inner)];
        }),
      );
      client.ws.send(encodeBatchFrame(data));

      // Each valid inner op is dispatched and acked; the last ack confirms the whole
      // frame was processed, and no 429 was raised for this legitimate batch.
      await waitForOpAck(client.messages, ids[ids.length - 1]);
      expect(sawError(client.messages, 429)).toBe(false);
      expect(hasExited()).toBe(false);

      client.close();
    });
  });
});
