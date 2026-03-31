/**
 * Cluster setup helper for 3-node Rust server integration tests.
 *
 * Spawns three server processes and manages their lifecycle for tests
 * that require a real multi-node cluster (partition map sync, failover, etc.).
 *
 * Each node uses a fixed port scheme:
 *   Node 0: WebSocket port 11001, cluster port 12001
 *   Node 1: WebSocket port 11002, cluster port 12002
 *   Node 2: WebSocket port 11003, cluster port 12003
 */

import * as child_process from 'child_process';
import * as path from 'path';
import * as readline from 'readline';

/** Repository root — three levels up from tests/integration-rust/helpers/. */
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');

/** Startup timeout — cluster formation takes longer than a single node start. */
const DEFAULT_CLUSTER_TIMEOUT_MS = 60_000;

const NODE_COUNT = 3;

/** Fixed port configuration for each cluster node. */
const NODE_CONFIGS = [
  { nodeId: 'node-0', wsPort: 11001, clusterPort: 12001 },
  { nodeId: 'node-1', wsPort: 11002, clusterPort: 12002 },
  { nodeId: 'node-2', wsPort: 11003, clusterPort: 12003 },
] as const;

export interface ClusterSetup {
  /** WebSocket ports for each node (index matches node index). */
  wsPorts: number[];
  /** Seed node addresses (cluster ports) for bootstrapping a ClusterClient. */
  seedAddresses: string[];
  /** Stop a specific node by index (0, 1, or 2). */
  stopNode: (index: number) => Promise<void>;
  /** Restart a previously stopped node. */
  restartNode: (index: number) => Promise<void>;
  /** Stop all nodes and clean up. */
  cleanup: () => Promise<void>;
}

/**
 * Spawns a 3-node Rust server cluster for integration testing.
 *
 * Waits for all three nodes to print PORT= before resolving, confirming
 * that their WebSocket handlers are ready.
 */
export async function spawnCluster(
  options: { timeout?: number } = {}
): Promise<ClusterSetup> {
  const timeoutMs = options.timeout ?? DEFAULT_CLUSTER_TIMEOUT_MS;
  const binaryPath = process.env.RUST_SERVER_BINARY;

  const processes: Array<child_process.ChildProcess | null> = new Array(NODE_COUNT).fill(null);

  function buildSeedNodes(excludeIndex: number): string {
    return NODE_CONFIGS.filter((_, i) => i !== excludeIndex)
      .map(c => `localhost:${c.clusterPort}`)
      .join(',');
  }

  function spawnNode(index: number): child_process.ChildProcess {
    const cfg = NODE_CONFIGS[index];
    const seedNodes = buildSeedNodes(index);

    const args = [
      '--node-id', cfg.nodeId,
      '--port', String(cfg.wsPort),
      '--cluster-port', String(cfg.clusterPort),
      '--seed-nodes', seedNodes,
    ];

    let proc: child_process.ChildProcess;

    if (binaryPath) {
      proc = child_process.spawn(binaryPath, args, {
        cwd: REPO_ROOT,
        detached: true,
        stdio: ['ignore', 'pipe', 'inherit'],
        env: { ...process.env },
      });
    } else {
      proc = child_process.spawn(
        'cargo',
        ['run', '--bin', 'test-server', '--release', '--', ...args],
        {
          cwd: REPO_ROOT,
          detached: true,
          stdio: ['ignore', 'pipe', 'inherit'],
          env: { ...process.env },
        }
      );
    }

    proc.unref();
    return proc;
  }

  // Spawn all three nodes in parallel
  for (let i = 0; i < NODE_COUNT; i++) {
    processes[i] = spawnNode(i);
  }

  // Wait for all nodes to signal readiness
  await Promise.all(
    processes.map((proc, i) => waitForPort(proc!, timeoutMs, NODE_CONFIGS[i].wsPort))
  );

  async function stopNode(index: number): Promise<void> {
    const proc = processes[index];
    if (!proc) return;
    processes[index] = null;
    await makeCleanup(proc)();
  }

  async function restartNode(index: number): Promise<void> {
    // Ensure any existing process is stopped first
    if (processes[index]) {
      await stopNode(index);
    }
    const proc = spawnNode(index);
    processes[index] = proc;
    await waitForPort(proc, timeoutMs, NODE_CONFIGS[index].wsPort);
  }

  async function cleanup(): Promise<void> {
    await Promise.all(
      processes.map((proc, i) => {
        if (!proc) return Promise.resolve();
        processes[i] = null;
        return makeCleanup(proc)();
      })
    );
  }

  return {
    wsPorts: NODE_CONFIGS.map(c => c.wsPort),
    seedAddresses: NODE_CONFIGS.map(c => `ws://localhost:${c.wsPort}/ws`),
    stopNode,
    restartNode,
    cleanup,
  };
}

// ---------------------------------------------------------------------------
// Internal helpers (mirrors patterns from tests/integration-rust/helpers/index.ts)
// ---------------------------------------------------------------------------

/**
 * Waits for the process to print PORT=<expectedPort> on stdout.
 * Rejects on timeout or premature process exit.
 */
function waitForPort(
  proc: child_process.ChildProcess,
  timeoutMs: number,
  expectedPort: number
): Promise<void> {
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
            `Cluster node (port ${expectedPort}) did not print PORT= within ${timeoutMs} ms. ` +
              `Set RUST_SERVER_BINARY to use a pre-built binary and skip cargo build time.`
          )
        )
      );
    }, timeoutMs);

    rl.on('line', (line) => {
      const match = /^PORT=(\d+)$/.exec(line.trim());
      if (match) {
        settle(() => resolve());
      }
    });

    proc.on('exit', (code) => {
      settle(() =>
        reject(
          new Error(
            `Cluster node (port ${expectedPort}) exited with code ${code} before printing PORT=`
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
 * SIGTERM, waits up to 5 s for a clean exit, then sends SIGKILL.
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

      const onExit = () => {
        clearTimeout(killTimer);
        resolve();
      };
      proc.once('exit', onExit);

      try {
        process.kill(-pid, 'SIGTERM');
      } catch {
        proc.removeListener('exit', onExit);
        resolve();
        return;
      }

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
