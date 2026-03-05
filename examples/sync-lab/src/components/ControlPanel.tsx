interface ControlPanelProps {
  readLatency: number;
  pendingOps: number;
  isConnected: boolean;
  deviceLabel: string;
}

/**
 * Magic Control Panel: shows real-time read latency, pending operations,
 * and sync status. Visible across all tabs.
 */
export function ControlPanel({
  readLatency,
  pendingOps,
  isConnected,
  deviceLabel,
}: ControlPanelProps) {
  const syncStatus =
    !isConnected
      ? 'Offline'
      : pendingOps === 0
        ? 'Synced'
        : `${pendingOps} ops pending`;

  return (
    <div className="flex items-center gap-4 rounded-lg bg-surface px-4 py-2 text-sm">
      <span className="font-medium text-text-muted">{deviceLabel}</span>

      {/* Connection status dot */}
      <span className="flex items-center gap-1.5">
        <span
          className={`inline-block h-2 w-2 rounded-full ${
            isConnected ? 'bg-success animate-pulse-dot' : 'bg-danger'
          }`}
        />
        <span className={isConnected ? 'text-success' : 'text-danger'}>
          {isConnected ? 'Online' : 'Offline'}
        </span>
      </span>

      {/* Read latency */}
      <span className="text-text-muted">
        Read:{' '}
        <span className="font-mono text-text">
          {readLatency < 1
            ? `${(readLatency * 1000).toFixed(0)}us`
            : `${readLatency.toFixed(2)}ms`}
        </span>
      </span>

      {/* Sync status */}
      <span className="text-text-muted">
        Sync:{' '}
        <span
          className={`font-mono ${
            syncStatus === 'Synced'
              ? 'text-success'
              : syncStatus === 'Offline'
                ? 'text-danger'
                : 'text-warning'
          }`}
        >
          {syncStatus}
        </span>
      </span>
    </div>
  );
}
