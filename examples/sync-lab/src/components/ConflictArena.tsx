import { useState } from 'react';
import { DevicePanel } from '@/components/DevicePanel';

/**
 * Tab 1: Conflict Arena — two side-by-side device panels sharing the
 * same todo list through the server. Demonstrates offline edits,
 * reconnect, and per-field HLC conflict resolution.
 */
export function ConflictArena() {
  const [showTimestamps, setShowTimestamps] = useState(false);

  return (
    <div>
      {/* Toolbar */}
      <div className="mb-4 flex items-center justify-between">
        <p className="text-sm text-text-muted">
          Two devices sharing one to-do list. Disconnect one, edit both, reconnect to see CRDT merge.
        </p>
        <label className="flex items-center gap-2 text-sm text-text-muted cursor-pointer">
          <input
            type="checkbox"
            checked={showTimestamps}
            onChange={e => setShowTimestamps(e.target.checked)}
            className="accent-primary"
          />
          Show HLC timestamps
        </label>
      </div>

      {/* Split-screen device panels */}
      <div className="grid gap-4 md:grid-cols-2">
        <DevicePanel
          deviceId="device-a"
          label="Device A"
          showTimestamps={showTimestamps}
        />
        <DevicePanel
          deviceId="device-b"
          label="Device B"
          showTimestamps={showTimestamps}
        />
      </div>
    </div>
  );
}
