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
 *
 * Each node is spawned with two fixed env overrides:
 *
 * `STORAGE_BACKEND=null` — uses the in-memory `NullDataStore` and avoids the
 * redb single-writer file lock at `./topgun.redb`. Three nodes spawned in
 * parallel from the same CWD would otherwise race on the file lock and fail
 * with `Database already open. Cannot acquire lock.`
 *
 * `TOPGUN_NO_AUTH=true` — disables JWT authentication on the server so the
 * `ClusterClient` (which connects without an auth token in these routing tests)
 * can reach the application-message stage of the WebSocket handler. Without
 * this, the server sits in the auth-handshake loop and silently drops every
 * non-AUTH frame — including `PARTITION_MAP_REQUEST` — so the client never
 * receives the partition map and `isRoutingActive()` never becomes true. These
 * tests exercise routing logic, not authentication, so no-auth is the correct
 * posture for this cluster.
 */

import * as child_process from 'child_process';
import * as path from 'path';
import * as readline from 'readline';
import { ClusterClient } from '@topgunbuild/client';

/** Repository root — three levels up from tests/integration-rust/helpers/. */
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');

/** Startup timeout — cluster formation takes longer than a single node start. */
const DEFAULT_CLUSTER_TIMEOUT_MS = 60_000;

const NODE_COUNT = 3;

/**
 * Budget for the post-restart join verification poll inside `restartNode()`.
 * The poll exits as soon as the cluster reports all NODE_COUNT members, so on
 * a fast machine this typically resolves in 1–3s. The 20s ceiling covers the
 * worst observed cold-build CPU-contention case where the seed-discovery →
 * TCP-peer-connect → JoinResponse → broadcast_partition_map chain stalls.
 */
const VERIFY_REJOIN_BUDGET_MS = 20_000;

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

  /**
   * Builds the `--seed-nodes` CLI arg for the node at `index`.
   *
   * `mode: 'boot'` — return self + lower-indexed peers (already spawned by the
   * sequential boot loop). The server's discover_seeds_and_join filters self,
   * so node-0 effectively sees an empty seed list and self-promotes; node-i
   * (i>0) sees nodes 0..i-1 and joins the existing cluster. We include self
   * (rather than passing empty) so `cluster_mode` stays true on node-0 — the
   * server skips ALL cluster machinery when seed_list is empty, which would
   * leave node-0 unable to accept later JoinRequests.
   *
   * Including all peers (the old behavior) caused split-brain at boot: all
   * three nodes spawn in parallel, each tries to contact the others before
   * any has self-promoted, every JoinRequest is rejected with "not master;
   * master address: unknown", and each falls through to self_promote — leaving
   * 3 independent single-node masters that never reliably merge. Downstream
   * tests then see partial membership views (Test 4's rejoin leaves master with
   * view={node-0, node-1} missing node-2; Test 5's failover times out waiting
   * for a mapVersion bump because the cluster never had 3 members to begin with).
   *
   * `mode: 'restart'` — return ALL other peers (excluding self). At restart time
   * the other peers are already running, so the rejoining node should contact
   * any of them to find the current master.
   */
  function buildSeedNodes(index: number, mode: 'boot' | 'restart'): string {
    if (mode === 'boot') {
      return NODE_CONFIGS.filter((_, i) => i <= index)
        .map(c => `localhost:${c.clusterPort}`)
        .join(',');
    }
    return NODE_CONFIGS.filter((_, i) => i !== index)
      .map(c => `localhost:${c.clusterPort}`)
      .join(',');
  }

  function spawnNode(index: number, mode: 'boot' | 'restart' = 'restart'): child_process.ChildProcess {
    const cfg = NODE_CONFIGS[index];
    const seedNodes = buildSeedNodes(index, mode);

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
        env: { ...process.env, STORAGE_BACKEND: 'null', TOPGUN_NO_AUTH: 'true' },
      });
    } else {
      proc = child_process.spawn(
        'cargo',
        ['run', '--bin', 'test-server', '--release', '--', ...args],
        {
          cwd: REPO_ROOT,
          detached: true,
          stdio: ['ignore', 'pipe', 'inherit'],
          env: { ...process.env, STORAGE_BACKEND: 'null', TOPGUN_NO_AUTH: 'true' },
        }
      );
    }

    proc.unref();
    return proc;
  }

  // Spawn nodes SEQUENTIALLY with per-node join verification. Parallel boot
  // causes a split-brain because each node simultaneously tries to contact the
  // others before any has self-promoted as master — every JoinRequest is rejected
  // with "not master; master address: unknown", and each node falls through to
  // self_promote_as_master. The cluster then has 3 independent single-node
  // masters that never reliably merge, which causes downstream tests to see
  // partial membership views (Test 4's rejoin leaves master with view={node-0,
  // node-1} missing node-2; Test 5's failover then times out waiting for a
  // mapVersion bump because the cluster's view never had 3 members to begin with).
  //
  // Sequential boot with post-spawn membership verification ensures node-0
  // self-promotes alone first, then node-1 joins it as a non-master peer, then
  // node-2 joins the now-2-node cluster. After each spawn we poll the cluster
  // via a temporary ClusterClient connected to already-confirmed nodes until
  // the broadcast partition map reports the expected nodeCount.
  for (let i = 0; i < NODE_COUNT; i++) {
    processes[i] = spawnNode(i, 'boot');
    await waitForPort(processes[i]!, timeoutMs, NODE_CONFIGS[i].wsPort);
    await waitForClusterMembership(i + 1, /* observerIndex */ 0);
  }

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
    // Wait until the cluster (observed via a stable peer) reports the restarted
    // node back in the membership. See waitForClusterMembership for rationale.
    const observerIndex = (index + 1) % NODE_COUNT;
    await waitForClusterMembership(NODE_COUNT, observerIndex);
  }

  /**
   * Polls the broadcast partition map until the cluster reports
   * `expectedNodeCount` members. Connects via a temporary ClusterClient to the
   * seed at `observerIndex` (which MUST already be a confirmed cluster member
   * with full membership view). The poll resolves as soon as the expected
   * nodeCount is reached, typically in 1–3s on a warm machine.
   *
   * Earlier iterations used fixed-sleep grace periods (2s → 5s → 8s) but
   * timing estimates remained flaky because the seed-discovery → TCP-peer-connect
   * → JoinResponse → MembershipReactor → broadcast_partition_map chain has no
   * deterministic upper bound under parallel-test CPU contention. Active
   * polling provides a positive correctness signal instead of an estimate.
   *
   * Connecting to the just-spawned node would let the verifier read that node's
   * own partition map (which sees only itself until seed gossip completes),
   * giving a false low nodeCount — so the observer MUST be a stable peer.
   */
  async function waitForClusterMembership(
    expectedNodeCount: number,
    observerIndex: number
  ): Promise<void> {
    const observerSeed = `ws://localhost:${NODE_CONFIGS[observerIndex].wsPort}/ws`;
    const verifier = new ClusterClient({
      enabled: true,
      seedNodes: [observerSeed],
      routingMode: 'direct',
    });
    try {
      await verifier.connect();
      const deadline = Date.now() + VERIFY_REJOIN_BUDGET_MS;
      while (Date.now() < deadline) {
        if (verifier.isRoutingActive()) {
          const stats = verifier.getRouterStats();
          if (stats && stats.nodeCount === expectedNodeCount) {
            return;
          }
        }
        await new Promise<void>(r => setTimeout(r, 100));
      }
      const finalStats = verifier.getRouterStats();
      throw new Error(
        `waitForClusterMembership: expected ${expectedNodeCount} members via ${observerSeed} within ${VERIFY_REJOIN_BUDGET_MS}ms ` +
          `(final nodeCount=${finalStats?.nodeCount ?? 'unknown'}, mapVersion=${finalStats?.mapVersion ?? 'unknown'})`
      );
    } finally {
      verifier.close();
    }
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
