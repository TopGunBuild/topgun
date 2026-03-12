import { useState, useCallback, useRef } from 'react';
import { LatencyHistogram } from '@/components/LatencyHistogram';
import { ControlPanel } from '@/components/ControlPanel';
import { useDeviceClient } from '@/hooks/useDeviceClient';
import { useLatencyTracker, type LatencyStats } from '@/hooks/useLatencyTracker';
import { prefixMap } from '@/lib/session';

const BENCHMARK_COUNT = 100;

/**
 * Tab 2: Latency Race — compare TopGun write latency online vs offline.
 * Runs a 100-write benchmark in each mode and displays side-by-side histograms.
 */
export function LatencyRace() {
  const { client, map, isConnected, disconnect, reconnect } = useDeviceClient('latency-racer');
  const { lastReadLatency, pendingOps, runBenchmark } = useLatencyTracker(map, client);

  const [onlineStats, setOnlineStats] = useState<LatencyStats | null>(null);
  const [offlineStats, setOfflineStats] = useState<LatencyStats | null>(null);
  const [running, setRunning] = useState(false);
  const reconnectRef = useRef(reconnect);
  reconnectRef.current = reconnect;

  const handleRunBenchmark = useCallback(async () => {
    if (!map || running) return;
    setRunning(true);

    // Phase 1: Online benchmark
    if (!isConnected) {
      reconnectRef.current();
    }
    // Small delay to let connection establish
    await new Promise(r => setTimeout(r, 300));

    const online = runBenchmark(map, BENCHMARK_COUNT);
    setOnlineStats(online);

    // Phase 2: Go offline, then benchmark
    disconnect();
    await new Promise(r => setTimeout(r, 100));

    // Need a new map reference after disconnect — use a temporary device
    // Since disconnect closes the client, we create a fresh offline one
    const { MemoryStorageAdapter } = await import('@/lib/memory-storage');
    const { TopGunClient } = await import('@topgunbuild/client');
    const storage = new MemoryStorageAdapter();
    const offlineClient = new TopGunClient({
      nodeId: 'latency-offline',
      storage,
      // Dummy URL — client initializes but fails to connect silently,
      // so writes still work locally (truly offline benchmark)
      serverUrl: 'ws://localhost:0',
    });
    const offlineMap = offlineClient.getMap<string, any>(prefixMap('latency-bench'));

    const offline = runBenchmark(offlineMap, BENCHMARK_COUNT);
    setOfflineStats(offline);

    offlineClient.close();

    // Reconnect the main device
    reconnectRef.current();

    setRunning(false);
  }, [map, running, isConnected, disconnect, runBenchmark]);

  return (
    <div>
      <p className="mb-4 text-sm text-text-muted">
        Compare write latency online vs offline. TopGun writes are sub-millisecond
        in both modes — because writes never wait for network.
      </p>

      {/* Control panel */}
      <ControlPanel
        readLatency={lastReadLatency}
        pendingOps={pendingOps}
        isConnected={isConnected}
        deviceLabel="latency-racer"
      />

      {/* Run benchmark button */}
      <div className="mt-4 flex items-center gap-4">
        <button
          onClick={handleRunBenchmark}
          disabled={running}
          className="rounded-lg bg-primary px-6 py-2.5 text-sm font-semibold text-white hover:bg-primary-dark disabled:opacity-50 transition-colors"
        >
          {running ? 'Running...' : `Run Benchmark (${BENCHMARK_COUNT} writes)`}
        </button>
        {running && (
          <span className="text-sm text-text-muted animate-pulse">
            Measuring latency...
          </span>
        )}
      </div>

      {/* Side-by-side histograms */}
      <div className="mt-6 grid gap-4 md:grid-cols-2">
        <LatencyHistogram
          stats={onlineStats}
          label="Online (connected to server)"
          color="#2563eb"
        />
        <LatencyHistogram
          stats={offlineStats}
          label="Offline (no server connection)"
          color="#22c55e"
        />
      </div>

      {/* Conclusion message */}
      {onlineStats && offlineStats && (
        <div className="mt-6 rounded-lg border border-primary/30 bg-primary/10 p-4 text-center">
          <p className="text-sm font-medium text-text">
            Online avg: <span className="font-mono text-primary">{onlineStats.avg.toFixed(3)}ms</span>
            {' | '}
            Offline avg: <span className="font-mono text-success">{offlineStats.avg.toFixed(3)}ms</span>
          </p>
          <p className="mt-2 text-sm text-text-muted">
            Both sub-millisecond — TopGun writes never block on network.
          </p>
        </div>
      )}
    </div>
  );
}
