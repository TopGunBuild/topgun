/**
 * An in-process TCP proxy that caps the server-to-client byte rate of each
 * connection, for tests of slow links.
 *
 * - Each accepted connection has its own budget: its server-to-client bytes
 *   are released in 10 ms ticks at the configured rate. Client-to-server bytes
 *   pass unthrottled.
 * - When more than `highWaterBytes` (default 256 KiB) waits on a connection,
 *   its upstream socket is paused, so the server meets TCP back-pressure as on
 *   a real slow link. A small value makes the proxy model a link rather than
 *   add a deep buffer of its own.
 * - A client that does not read stops its connection: while the client
 *   socket refuses more bytes (`write` returns false), nothing more is
 *   released to it until it drains, so the pending bytes pass the high-water
 *   mark and the server meets back-pressure, as it would from that client.
 * - When either side of a connection ends or errors, the other side is
 *   destroyed and the connection's pending bytes are dropped, so an abandoned
 *   connection does not keep the server sending.
 * - `blackhole()` turns every existing connection into a dead path: bytes are
 *   dropped both ways and closes are no longer propagated, while both sockets
 *   stay open. Connections accepted later forward normally, as after a network
 *   switch.
 *
 * It listens without a host, so Node binds the dual-stack address and a client
 * dialing `localhost` reaches it whether that resolves to `::1` or `127.0.0.1`.
 */
import * as net from 'net';

export interface ThrottleProxy {
  port: number;
  /** Server-to-client bytes written to clients so far, over every connection. */
  bytesDownstream(): number;
  /** The local ports of the open connections' upstream sockets, to find them in `ss`. */
  upstreamPorts(): number[];
  /** Sets the cap of existing and later connections; `Infinity` is unthrottled. */
  setRate(bitsPerSecond: number): void;
  blackhole(): void;
  close(): Promise<void>;
}

const TICK_MS = 10;
const DEFAULT_HIGH_WATER = 256 * 1024;
/** The most credit a connection keeps across idle ticks, so it never bursts above the rate. */
const MAX_CREDIT_TICKS = 5;

interface Link {
  client: net.Socket;
  upstream: net.Socket;
  pending: Buffer[];
  pendingBytes: number;
  credit: number;
  /** The client socket refused more bytes; released again on its `'drain'`. */
  blocked: boolean;
  dead: boolean;
  closed: boolean;
}

export async function startThrottleProxy(options: {
  targetPort: number;
  bitsPerSecond: number;
  highWaterBytes?: number;
}): Promise<ThrottleProxy> {
  let rate = options.bitsPerSecond;
  const highWater = options.highWaterBytes ?? DEFAULT_HIGH_WATER;
  let downstream = 0;
  const links = new Set<Link>();

  const shut = (link: Link) => {
    if (link.closed) return;
    link.closed = true;
    link.pending = [];
    link.pendingBytes = 0;
    links.delete(link);
    link.client.destroy();
    link.upstream.destroy();
  };

  const deliver = (link: Link, bytes: Buffer) => {
    downstream += bytes.length;
    if (!link.client.write(bytes)) {
      link.blocked = true;
      link.client.once('drain', () => {
        link.blocked = false;
      });
    }
  };

  const pump = (link: Link, elapsedMs: number) => {
    if (link.dead || link.closed || link.blocked) return;
    if (rate === Infinity) {
      while (link.pending.length > 0 && !link.blocked) {
        const bytes = link.pending.shift()!;
        link.pendingBytes -= bytes.length;
        deliver(link, bytes);
      }
    } else {
      const perMs = rate / 8 / 1000;
      link.credit = Math.min(link.credit + perMs * elapsedMs, perMs * TICK_MS * MAX_CREDIT_TICKS);
      while (link.pending.length > 0 && link.credit >= 1 && !link.blocked) {
        const head = link.pending[0];
        const take = Math.min(head.length, Math.floor(link.credit));
        deliver(link, head.subarray(0, take));
        link.credit -= take;
        link.pendingBytes -= take;
        if (take === head.length) link.pending.shift();
        else link.pending[0] = head.subarray(take);
      }
    }
    if (link.pendingBytes <= highWater && link.upstream.isPaused()) link.upstream.resume();
  };

  let last = Date.now();
  const timer = setInterval(() => {
    const now = Date.now();
    const elapsed = now - last;
    last = now;
    for (const link of links) pump(link, elapsed);
  }, TICK_MS);

  const server = net.createServer((client) => {
    const upstream = net.connect({ port: options.targetPort, host: 'localhost' });
    const link: Link = {
      client,
      upstream,
      pending: [],
      pendingBytes: 0,
      credit: 0,
      blocked: false,
      dead: false,
      closed: false,
    };
    links.add(link);
    client.setNoDelay(true);
    upstream.setNoDelay(true);

    upstream.on('data', (bytes: Buffer) => {
      if (link.dead || link.closed) return;
      if (rate === Infinity && link.pending.length === 0 && !link.blocked) {
        deliver(link, bytes);
        return;
      }
      link.pending.push(bytes);
      link.pendingBytes += bytes.length;
      if (link.pendingBytes > highWater) upstream.pause();
    });
    client.on('data', (bytes: Buffer) => {
      if (link.dead || link.closed) return;
      upstream.write(bytes);
    });
    for (const socket of [client, upstream]) {
      socket.on('end', () => {
        if (!link.dead) shut(link);
      });
      socket.on('close', () => {
        if (!link.dead) shut(link);
      });
      socket.on('error', () => {
        if (!link.dead) shut(link);
      });
    }
  });

  await new Promise<void>((resolve) => server.listen(0, resolve));
  const address = server.address() as net.AddressInfo;

  return {
    port: address.port,
    bytesDownstream: () => downstream,
    upstreamPorts: () =>
      [...links].map((link) => link.upstream.localPort).filter((p): p is number => p !== undefined),
    setRate: (bitsPerSecond: number) => {
      rate = bitsPerSecond;
    },
    blackhole: () => {
      for (const link of links) {
        link.dead = true;
        link.pending = [];
        link.pendingBytes = 0;
        // Not reading the server's bytes any more lets its TCP window fill,
        // as it would on a path that delivers nothing.
        link.upstream.pause();
      }
    },
    close: async () => {
      clearInterval(timer);
      for (const link of [...links]) {
        link.dead = false;
        shut(link);
      }
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
