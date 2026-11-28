/**
 * A TopGunClient whose credentials the test drives, run in a process of its
 * own by helpers/auth-client-process.js, and {@link AuthClient}, the test's
 * handle on that process.
 *
 * The process is started with `[port, JSON.stringify(start)]`,
 * `start` being `{ provider: token }` (a token provider that returns `token`)
 * or `{ token }` (a static token). Storage is in memory, so the client holds
 * no device credential from an earlier process. Every 100 ms it tells the
 * parent `{ type: 'status', … }`:
 *
 * - `state` and `changes` — the connection state and every transition, each
 *   with `at` (ms since the epoch);
 * - `auths` — every `AUTH` frame the client sent, `{ token, at }`;
 * - `acks` / `fails` — every `AUTH_ACK` (`{ at }`) and `AUTH_FAIL`
 *   (`{ code, at }`) it received;
 * - `providerCalls` — when each token provider call was made;
 * - `sockets` — how many sockets the client opened;
 * - `querySubs` — how many `QUERY_SUB` frames it sent;
 * - `authRequired` — how many times `onAuthRequired` was called;
 * - `deviceId` — the device id the server bound;
 * - `pending` — its pending writes; `opAcks` — the `OP_ACK`s received;
 * - `rejections` — every write reported on `onWriteRejected`.
 *
 * Commands from the parent:
 *
 * - `{ type: 'providerToken', token }` — what the current provider returns from now on;
 * - `{ type: 'setProvider', token }` — `setAuthTokenProvider` with a new provider returning `token`;
 * - `{ type: 'setToken', token }` — `setAuthToken(token)`;
 * - `{ type: 'write', map, key, value }` — a write of `value` at `map`/`key`;
 * - `{ type: 'watch', map }` — a live query of `map`;
 * - `{ type: 'exit' }` — closes the client and ends the process.
 */
import * as child_process from 'child_process';
import * as path from 'path';

import { TopGunClient } from '@topgunbuild/client';
import type { StateChangeEvent } from '@topgunbuild/client';
import type { WriteRejection } from '@topgunbuild/core';

import { MemoryStorageAdapter } from './memory-storage';

export interface AuthClientStatus {
  state: string;
  changes: Array<{ from: string; to: string; at: number }>;
  auths: Array<{ token: string; at: number }>;
  acks: Array<{ at: number }>;
  fails: Array<{ code?: number; at: number }>;
  providerCalls: number[];
  sockets: number;
  querySubs: number;
  authRequired: number;
  deviceId: string | null;
  pending: number;
  opAcks: number;
  rejections: Array<{ id: string; mapName: string; key: string; cause: string }>;
}

/** The test's handle on one auth-client process. */
export class AuthClient {
  private readonly proc: child_process.ChildProcess;
  private latest: AuthClientStatus | null = null;

  constructor(port: number, start: { provider: string } | { token: string }) {
    this.proc = child_process.fork(
      path.join(__dirname, 'auth-client-process.js'),
      [String(port), JSON.stringify(start)],
      { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] },
    );
    this.proc.on('message', (message: { type?: string }) => {
      if (message?.type === 'status') this.latest = message as unknown as AuthClientStatus;
    });
  }

  /** The last status the process reported. */
  get status(): AuthClientStatus {
    if (!this.latest) throw new Error('the auth client has not reported yet');
    return this.latest;
  }

  send(message: Record<string, unknown>): void {
    this.proc.send(message);
  }

  /** Resolves once `probe` holds for a reported status; rejects after `ceilingMs`. */
  async until(
    what: string,
    probe: (s: AuthClientStatus) => boolean,
    ceilingMs: number,
  ): Promise<AuthClientStatus> {
    const deadline = Date.now() + ceilingMs;
    for (;;) {
      if (this.latest && probe(this.latest)) return this.latest;
      if (Date.now() >= deadline) {
        throw new Error(
          `${what}: not within ${ceilingMs} ms; last status ${JSON.stringify(this.latest)}`,
        );
      }
      await new Promise((r) => setTimeout(r, 25));
    }
  }

  /** Lets the next status in, so it reflects everything sent before this call. */
  async fresh(): Promise<AuthClientStatus> {
    this.latest = null;
    return this.until('a fresh status', () => true, 5_000);
  }

  async exit(): Promise<void> {
    if (this.proc.exitCode !== null || this.proc.signalCode !== null) return;
    const exited = new Promise((r) => this.proc.once('exit', r));
    this.proc.send({ type: 'exit' });
    const timer = setTimeout(() => this.proc.kill('SIGKILL'), 5_000);
    await exited;
    clearTimeout(timer);
  }
}

if (process.env.TOPGUN_AUTH_CLIENT_CHILD === '1') runChild();

function runChild(): void {
  const [port, startJson] = process.argv.slice(2);
  const start = JSON.parse(startJson) as { provider?: string; token?: string };

  const changes: Array<{ from: string; to: string; at: number }> = [];
  const auths: Array<{ token: string; at: number }> = [];
  const acks: Array<{ at: number }> = [];
  const fails: Array<{ code?: number; at: number }> = [];
  const providerCalls: number[] = [];
  const rejections: Array<Pick<WriteRejection, 'id' | 'mapName' | 'key' | 'cause'>> = [];
  let sockets = 0;
  let querySubs = 0;
  let authRequired = 0;
  let opAcks = 0;
  let providerToken: string | null = start.provider ?? null;

  const provider = () => async () => {
    providerCalls.push(Date.now());
    return providerToken;
  };

  const client = new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    onAuthRequired: () => {
      authRequired += 1;
    },
  });

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the helper reaches into the client's engine
  const engine = (client as any).syncEngine;
  client.onConnectionStateChange((event: StateChangeEvent) =>
    changes.push({ from: event.from, to: event.to, at: Date.now() }),
  );
  engine.webSocketManager.getConnectionProvider().on('connected', () => {
    sockets += 1;
  });
  const sendMessage = engine.webSocketManager.sendMessage.bind(engine.webSocketManager);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- any outbound frame
  engine.webSocketManager.sendMessage = (message: any, key?: string) => {
    if (message?.type === 'AUTH') auths.push({ token: message.token, at: Date.now() });
    if (message?.type === 'QUERY_SUB') querySubs += 1;
    return sendMessage(message, key);
  };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- frames on the engine's bus are untyped
  engine.on('message', (message: any) => {
    if (message?.type === 'AUTH_ACK') acks.push({ at: Date.now() });
    if (message?.type === 'AUTH_FAIL') fails.push({ code: message.code, at: Date.now() });
    if (message?.type === 'OP_ACK') opAcks += 1;
  });
  client.onWriteRejected((r) =>
    rejections.push({ id: r.id, mapName: r.mapName, key: r.key, cause: r.cause }),
  );

  if (start.provider !== undefined) client.setAuthTokenProvider(provider());
  if (start.token !== undefined) client.setAuthToken(start.token);

  const timer = setInterval(
    () =>
      process.send?.({
        type: 'status',
        state: client.getConnectionState(),
        changes,
        auths,
        acks,
        fails,
        providerCalls,
        sockets,
        querySubs,
        authRequired,
        deviceId: engine.deviceId ?? null,
        pending: client.getPendingOpsCount(),
        opAcks,
        rejections,
      }),
    100,
  );

  process.on(
    'message',
    (message: { type?: string; token?: string; map?: string; key?: string; value?: unknown }) => {
      switch (message?.type) {
        case 'providerToken':
          providerToken = message.token ?? null;
          break;
        case 'setProvider':
          providerToken = message.token ?? null;
          client.setAuthTokenProvider(provider());
          break;
        case 'setToken':
          client.setAuthToken(message.token!);
          break;
        case 'write':
          client.getMap(message.map!).set(message.key!, message.value);
          break;
        case 'watch':
          client.query(message.map!, {}).subscribe(() => undefined);
          break;
        case 'exit':
          clearInterval(timer);
          void Promise.resolve(client.close()).finally(() => process.exit(0));
          break;
        default:
          break;
      }
    },
  );

  void client.start();
}
