/**
 * Test harness for Rust server integration tests.
 *
 * Provides three helpers:
 * - spawnRustServer()       — starts the Rust test binary, captures the port
 * - createRustTestClient()  — connects a WebSocket client to /ws, auto-authenticates
 * - createRustTestContext() — spawns server + N clients, waits for all AUTH_ACK
 */

import * as child_process from 'child_process';
import * as path from 'path';
import * as readline from 'readline';

import { createTestClient, TestClient } from './test-client';

/** Repository root — two levels up from tests/integration-rust/helpers/. */
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');

/** Default timeout (ms) for server startup, including potential cargo build time. */
const DEFAULT_SERVER_TIMEOUT_MS = 30_000;

export interface SpawnedServer {
  port: number;
  process: child_process.ChildProcess;
  cleanup: () => Promise<void>;
}

export interface RustTestContext {
  port: number;
  clients: TestClient[];
  cleanup: () => Promise<void>;
}

/**
 * Spawns the Rust test server binary and waits for it to print `PORT=<number>`
 * to stdout.
 *
 * By default this runs `cargo run --bin test-server --release` from the
 * repository root, which will trigger a cargo build on the first invocation.
 * In CI, set the `RUST_SERVER_BINARY` environment variable to the path of a
 * pre-built binary to skip the cargo overhead and get a single-process launch
 * (which makes cleanup more reliable — no cargo wrapper process tree).
 *
 * stderr from the spawned process is inherited by the parent so that cargo
 * build output and Rust tracing logs are visible in the test output. stderr
 * is NOT parsed for the PORT protocol — only stdout is used.
 *
 * Process cleanup:
 * 1. SIGTERM is sent to the process group (negated PID) to propagate to the
 *    binary even when launched via `cargo run` (which spawns a child process).
 * 2. After 5 s, if the process is still alive, SIGKILL is sent.
 */
export async function spawnRustServer(
  options: {
    timeout?: number;
    env?: Record<string, string>;
  } = {}
): Promise<SpawnedServer> {
  const timeoutMs =
    options.timeout ??
    (process.env.RUST_SERVER_TIMEOUT
      ? parseInt(process.env.RUST_SERVER_TIMEOUT, 10)
      : DEFAULT_SERVER_TIMEOUT_MS);

  // Allow overriding the binary path in CI for reliable single-process cleanup
  const binaryPath = process.env.RUST_SERVER_BINARY;

  let proc: child_process.ChildProcess;

  if (binaryPath) {
    // Pre-built binary: single process, no cargo wrapper
    proc = child_process.spawn(binaryPath, [], {
      cwd: REPO_ROOT,
      detached: true,
      stdio: ['ignore', 'pipe', 'inherit'],
      env: { ...process.env, ...options.env },
    });
  } else {
    // Development: let cargo build and run the binary
    proc = child_process.spawn(
      'cargo',
      ['run', '--bin', 'test-server', '--release'],
      {
        cwd: REPO_ROOT,
        detached: true,
        stdio: ['ignore', 'pipe', 'inherit'],
        env: { ...process.env, ...options.env },
      }
    );
  }

  // Unref so that the test process itself is not kept alive by the child
  proc.unref();

  const port = await waitForPort(proc, timeoutMs);

  const cleanup = makeCleanup(proc);

  return { port, process: proc, cleanup };
}

/**
 * Creates a test client connecting to ws://localhost:${port}/ws.
 *
 * The `/ws` path is where the Rust test server mounts its WebSocket upgrade
 * handler. Auto-authentication is enabled by default.
 */
export async function createRustTestClient(
  port: number,
  options: {
    nodeId?: string;
    autoAuth?: boolean;
    userId?: string;
    roles?: string[];
  } = {}
): Promise<TestClient> {
  const serverUrl = `ws://localhost:${port}/ws`;
  return createTestClient(serverUrl, options);
}

/**
 * Spawns a Rust server and connects `numClients` clients to it.
 * Waits for all clients to receive AUTH_ACK before returning.
 * The returned `cleanup` function closes all WebSockets and kills the server.
 */
export async function createRustTestContext(
  numClients = 1,
  options: {
    clientOptions?: Parameters<typeof createRustTestClient>[1];
    serverOptions?: Parameters<typeof spawnRustServer>[0];
  } = {}
): Promise<RustTestContext> {
  const { port, cleanup: killServer } = await spawnRustServer(
    options.serverOptions
  );

  const clients: TestClient[] = [];
  try {
    for (let i = 0; i < numClients; i++) {
      const client = await createRustTestClient(port, {
        nodeId: `client-${i}`,
        userId: `user-${i}`,
        roles: ['ADMIN'],
        ...options.clientOptions,
      });
      clients.push(client);
    }

    // Wait for all clients to complete the auth handshake
    await Promise.all(clients.map((c) => c.waitForMessage('AUTH_ACK', 10_000)));
  } catch (err) {
    // Clean up on error so no processes are left dangling
    for (const client of clients) {
      client.close();
    }
    await killServer();
    throw err;
  }

  const cleanup = async () => {
    for (const client of clients) {
      client.close();
    }
    await killServer();
  };

  return { port, clients, cleanup };
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Reads stdout line by line and resolves with the port number once
 * `PORT=<number>` is seen.  Rejects if the timeout elapses first or if the
 * process exits before printing the port.
 */
function waitForPort(
  proc: child_process.ChildProcess,
  timeoutMs: number
): Promise<number> {
  return new Promise((resolve, reject) => {
    const rl = readline.createInterface({ input: proc.stdout! });

    let settled = false;
    const settle = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      rl.close();
      fn();
    };

    const timer = setTimeout(() => {
      settle(() =>
        reject(
          new Error(
            `Rust server did not print PORT= within ${timeoutMs} ms. ` +
              `Set RUST_SERVER_BINARY to use a pre-built binary and skip cargo build time.`
          )
        )
      );
    }, timeoutMs);

    rl.on('line', (line) => {
      const match = /^PORT=(\d+)$/.exec(line.trim());
      if (match) {
        const port = parseInt(match[1], 10);
        settle(() => resolve(port));
      }
    });

    proc.on('exit', (code) => {
      settle(() =>
        reject(
          new Error(
            `Rust server process exited with code ${code} before printing PORT=`
          )
        )
      );
    });

    proc.on('error', (err) => {
      settle(() => reject(err));
    });
  });
}

/**
 * Returns an async cleanup function that terminates the process group with
 * SIGTERM, waits up to 5 s for a clean exit, and then sends SIGKILL.
 *
 * The negated PID (`-pid`) is used to kill the entire process group, which
 * ensures that the actual binary is killed even when it was launched via
 * `cargo run` (which spawns a child process under the cargo wrapper).
 */
function makeCleanup(proc: child_process.ChildProcess): () => Promise<void> {
  return () =>
    new Promise<void>((resolve) => {
      if (proc.exitCode !== null || proc.killed) {
        resolve();
        return;
      }

      const pid = proc.pid;
      if (pid == null) {
        resolve();
        return;
      }

      // Wait for the process to exit on its own after the signal
      const onExit = () => {
        clearTimeout(killTimer);
        resolve();
      };
      proc.once('exit', onExit);

      // SIGTERM to the process group so cargo child processes are also stopped
      try {
        process.kill(-pid, 'SIGTERM');
      } catch {
        // Process may have already exited; resolve immediately
        proc.removeListener('exit', onExit);
        resolve();
        return;
      }

      // Escalate to SIGKILL if the process has not exited within 5 s
      const killTimer = setTimeout(() => {
        proc.removeListener('exit', onExit);
        try {
          process.kill(-pid, 'SIGKILL');
        } catch {
          // Already gone
        }
        resolve();
      }, 5_000);
    });
}

export { TestClient } from './test-client';
export { createTestToken, waitForSync, waitUntil } from './test-client';
