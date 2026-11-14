/**
 * What the tests that drive a `fault-injection` server build have in common:
 * how the binary is required, how a write is shaped, and how the per-partition
 * WAL lag gauge and the unlabelled series are read.
 *
 * No release build has the `fault-injection` cargo feature, so the binary is
 * given by path (FAULT_SERVER_BINARY) together with the commit it was built
 * from (FAULT_SERVER_COMMIT). With either missing a test fails; it never skips
 * and never falls back to another binary.
 */

import * as fs from 'fs';

export const FAULT_BUILD_COMMAND =
  'cargo build --release -p topgun-server --features fault-injection --target-dir target/fault-injection';

/** Every seam of a `fault-injection` build announces itself at boot with a line that starts like this. */
export const SEAM_BOOT_TEXT = 'fault-injection build';

export const LAG_GAUGE = 'topgun_wal_applied_watermark_lag';

export interface FaultBinary {
  binary: string;
  commit: string;
}

/** Reads the two variables that name the seam binary, and throws — never skips — when either is unusable. */
export function requireFaultBinary(): FaultBinary {
  const hint = `Build the server with the store-refusal seam (\`${FAULT_BUILD_COMMAND}\`) and export FAULT_SERVER_BINARY (its path) and FAULT_SERVER_COMMIT (the commit it was built from).`;
  const binary = process.env.FAULT_SERVER_BINARY;
  if (!binary) throw new Error(`FAULT_SERVER_BINARY is not set. ${hint}`);
  const commit = process.env.FAULT_SERVER_COMMIT;
  if (!commit) throw new Error(`FAULT_SERVER_COMMIT is not set. ${hint}`);
  try {
    if (!fs.statSync(binary).isFile()) throw new Error('not a regular file');
    fs.accessSync(binary, fs.constants.X_OK);
  } catch (err) {
    throw new Error(
      `FAULT_SERVER_BINARY does not name an executable file (${binary}: ${(err as Error).message}). ${hint}`,
    );
  }
  return { binary, commit };
}

/** The payload of one last-write-wins PUT, stamped with the sender's clock. */
export function lwwPut(
  id: string,
  mapName: string,
  key: string,
  value: unknown,
): Record<string, unknown> {
  return {
    id,
    mapName,
    opType: 'PUT',
    key,
    record: { value, timestamp: { millis: Date.now(), counter: 0, nodeId: 'liveness' } },
  };
}

/** One scrape of the lag gauge: partition id to lag. */
export type LagScrape = Record<string, number>;

export async function scrapeLag(port: number): Promise<LagScrape> {
  const resp = await fetch(`http://localhost:${port}/metrics`);
  const body = await resp.text();
  const lags: LagScrape = {};
  for (const line of body.split('\n')) {
    if (!line.startsWith(`${LAG_GAUGE}{`)) continue;
    const partition = /partition="(\d+)"/.exec(line);
    const value = Number(line.trim().split(/\s+/).pop());
    if (partition && Number.isFinite(value)) lags[partition[1]] = value;
  }
  return lags;
}

/**
 * Reads one series that has no labels, or `null` when the server does not
 * render it: a build without the series is told that way from one that
 * reports zero.
 */
export async function scrapeGauge(port: number, name: string): Promise<number | null> {
  return seriesValue(await scrapeMetrics(port), name);
}

async function scrapeMetrics(port: number): Promise<string> {
  const resp = await fetch(`http://localhost:${port}/metrics`);
  return resp.text();
}

/** The value of an unlabelled series in one `/metrics` body, or `null` when it is not rendered. */
function seriesValue(body: string, name: string): number | null {
  for (const line of body.split('\n')) {
    const [series, sample] = line.trim().split(/\s+/);
    if (series !== name) continue;
    const value = Number(sample);
    return Number.isFinite(value) ? value : null;
  }
  return null;
}

/** True for a scrape that has at least one sample and every sample at zero; an empty scrape says nothing. */
export function allZero(scrape: LagScrape): boolean {
  const values = Object.values(scrape);
  return values.length > 0 && values.every((lag) => lag === 0);
}

/** Entries the committed write-behind passes stored; moved only by a pass that stores something. */
export const FLUSH_PASS_ENTRIES = 'topgun_write_behind_flush_pass_entries_total';
/** Buffered write-behind keys, set at the end of every pass. */
export const BUFFERED_ENTRIES = 'topgun_write_behind_buffered_entries';
/**
 * How long after a scrape that shows the early writes stored the lag scrape
 * that ends step 2 may be taken: one 600 ms refresh of the lag gauge (a stall
 * bound of 6000 ms) and one poll, so the sample it reads postdates the store.
 */
export const LAG_AFTER_STORED_MS = 700;

/** One reading of the two write-behind series step 2 reads. */
export interface FlushReading {
  entries: number | null;
  buffered: number | null;
}

async function withinMs<T>(work: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} did not answer within ${ms} ms`)), ms);
  });
  try {
    return await Promise.race([work, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/** Both series from one scrape, so they come from the same pass end. */
async function readFlush(port: number, scrapeWaitMs: number): Promise<FlushReading> {
  const body = await withinMs(
    scrapeMetrics(port),
    scrapeWaitMs,
    `scrape of /metrics on port ${port}`,
  );
  return {
    entries: seriesValue(body, FLUSH_PASS_ENTRIES),
    buffered: seriesValue(body, BUFFERED_ENTRIES),
  };
}

/** Timing of the step-2 waits; every field is in milliseconds. */
export interface FlushWaitTiming {
  pollMs: number;
  scrapeWaitMs: number;
}

/**
 * Waits until the write-behind buffer is idle and returns the stored-entries
 * counter then, the baseline `E0` of step 2, or why it was not reached.
 *
 * Idle: two consecutive scrapes with nothing buffered and the same counter.
 * The buffered gauge is set only at the end of a pass, so the first scrape of
 * the pair counts only once `spanMs` — at least one flush interval of the
 * server — has passed since `sinceAt`, when the arm's connection opened: a
 * pass has then run after the record the connection wrote, and `E0` is not
 * one short. Two reads keep a baseline taken between a pass's two metric
 * updates from counting.
 */
export async function waitIdleBaseline(
  port: number,
  sinceAt: number,
  spanMs: number,
  deadlineAt: number,
  timing: FlushWaitTiming,
): Promise<{ e0: number | null; detail: string }> {
  let previous: (FlushReading & { at: number }) | null = null;
  let last: FlushReading = { entries: null, buffered: null };
  while (Date.now() < deadlineAt) {
    const at = Date.now();
    last = await readFlush(port, timing.scrapeWaitMs);
    if (
      previous !== null &&
      previous.at - sinceAt >= spanMs &&
      previous.buffered === 0 &&
      last.buffered === 0 &&
      last.entries !== null &&
      previous.entries === last.entries
    ) {
      return {
        e0: last.entries,
        detail: `idle after ${at - sinceAt} ms: ${FLUSH_PASS_ENTRIES}=${last.entries} ${BUFFERED_ENTRIES}=0`,
      };
    }
    previous = { ...last, at };
    await new Promise((resolve) => setTimeout(resolve, timing.pollMs));
  }
  return {
    e0: null,
    detail: `not idle within ${deadlineAt - sinceAt} ms of the connection: last ${FLUSH_PASS_ENTRIES}=${last.entries} ${BUFFERED_ENTRIES}=${last.buffered}`,
  };
}

/**
 * Step 2's flush sign: after the last early acknowledgement, a scrape whose
 * stored-entries counter has risen by at least `count` over `e0` while nothing
 * is buffered, and then, no earlier than `LAG_AFTER_STORED_MS` later, a lag
 * scrape with every sample at zero.
 *
 * It cannot pass while an early write is unstored: that write keeps the
 * buffered count above 0 once a pass has ended, and its partition's lag above
 * 0 in every sample. A counter that never rises fails the first conjunct.
 */
export async function waitEarlyWritesStored(
  port: number,
  e0: number,
  count: number,
  lastAckAt: number,
  deadlineAt: number,
  timing: FlushWaitTiming,
): Promise<{ storedAfterMs: number | null; flushedAfterMs: number | null; detail: string }> {
  let storedAfterMs: number | null = null;
  let flushedAfterMs: number | null = null;
  let reading: FlushReading = { entries: null, buffered: null };
  let lag: LagScrape = {};
  while (flushedAfterMs === null && Date.now() < deadlineAt) {
    if (storedAfterMs === null) {
      reading = await readFlush(port, timing.scrapeWaitMs);
      if (reading.entries !== null && reading.entries >= e0 + count && reading.buffered === 0) {
        storedAfterMs = Date.now() - lastAckAt;
        await new Promise((resolve) => setTimeout(resolve, LAG_AFTER_STORED_MS));
        continue;
      }
    } else {
      lag = await withinMs(
        scrapeLag(port),
        timing.scrapeWaitMs,
        `scrape of /metrics on port ${port}`,
      );
      if (allZero(lag)) {
        flushedAfterMs = Date.now() - lastAckAt;
        break;
      }
    }
    await new Promise((resolve) => setTimeout(resolve, timing.pollMs));
  }
  return {
    storedAfterMs,
    flushedAfterMs,
    detail:
      `${FLUSH_PASS_ENTRIES}=${reading.entries} (E0=${e0}, needs ${e0 + count}) ` +
      `${BUFFERED_ENTRIES}=${reading.buffered} stored after ${storedAfterMs} ms; ` +
      `last lag scrape ${JSON.stringify(lag)}, all zero after ${flushedAfterMs} ms`,
  };
}
