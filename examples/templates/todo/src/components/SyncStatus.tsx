import { useState, useEffect } from 'react';
import { useClient } from '@topgunbuild/react';
import { SyncState } from '@topgunbuild/client';

/**
 * Primary differentiator banner. Shows the connection state so a first-time
 * visitor can see the "offline → syncing → connected" transition in real time.
 *
 * Pending-ops count: we poll the client's connection state. When the client
 * reconnects after being offline, the SyncEngine drains its op-log and
 * transitions through SYNCING → CONNECTED. We surface that as "syncing…"
 * which is immediately legible without CRDT mental-model scaffolding.
 */
export function SyncStatus() {
  const client = useClient();
  const [state, setState] = useState<SyncState>(client.getConnectionState());
  const [mergeCount, setMergeCount] = useState(0);

  useEffect(() => {
    const unsub = client.onConnectionStateChange((event) => {
      setState(event.to);
      // Each time the client finishes syncing, bump the visible "merged writes"
      // counter so the banner reads "merged N pending writes" momentarily.
      if (event.from === SyncState.SYNCING && event.to === SyncState.CONNECTED) {
        setMergeCount((n) => n + 1);
      }
    });
    return unsub;
  }, [client]);

  const label = stateLabel(state, mergeCount);
  const color = stateColor(state);

  return (
    <div className={`px-4 py-2 text-sm font-medium rounded-md ${color}`}>
      {label}
    </div>
  );
}

function stateLabel(state: SyncState, mergeCount: number): string {
  switch (state) {
    case SyncState.CONNECTED:
      return mergeCount > 0
        ? `Synced · merged ${mergeCount} pending ${mergeCount === 1 ? 'write' : 'writes'}`
        : 'Connected';
    case SyncState.SYNCING:
      return 'Syncing…';
    case SyncState.DISCONNECTED:
    case SyncState.BACKOFF:
      return 'Offline · writes queued locally';
    case SyncState.CONNECTING:
    case SyncState.AUTHENTICATING:
      return 'Connecting…';
    default:
      return 'Initialising…';
  }
}

function stateColor(state: SyncState): string {
  switch (state) {
    case SyncState.CONNECTED:
      return 'bg-green-100 text-green-800';
    case SyncState.SYNCING:
      return 'bg-blue-100 text-blue-800';
    case SyncState.DISCONNECTED:
    case SyncState.BACKOFF:
      return 'bg-amber-100 text-amber-800';
    default:
      return 'bg-gray-100 text-gray-600';
  }
}
