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
  const resp = await fetch(`http://localhost:${port}/metrics`);
  const body = await resp.text();
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
