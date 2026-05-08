import { useState, useCallback } from 'react';
import { DevicePanel } from '@/components/DevicePanel';
import { GuidedTour } from '@/components/GuidedTour';
import { MergeBanner } from '@/components/MergeBanner';

/**
 * Tab 1: Conflict Arena — two side-by-side device panels sharing the
 * same todo list through the server. Demonstrates offline edits,
 * reconnect, and per-field HLC conflict resolution.
 */
export function ConflictArena() {
  const [showTimestamps, setShowTimestamps] = useState(false);

  // MergeBanner state — re-keyed on each new merge to reset the 8s auto-dismiss timer
  const [bannerCount, setBannerCount] = useState(0);
  const [bannerKey, setBannerKey] = useState(0);

  // ?embed and ?demo params control narrative overlays
  const params = new URLSearchParams(window.location.search);
  const embed = params.has('embed');
  const demo = params.has('demo');

  const handleMergeDetected = useCallback((count: number) => {
    if (count <= 0) return;
    setBannerCount(count);
    // Increment key so React re-mounts the banner, resetting its internal timer
    setBannerKey(k => k + 1);
  }, []);

  return (
    <div>
      {/* Guided tour overlay on first visit — hidden in embed and demo modes */}
      {!embed && !demo && <GuidedTour />}

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

      {/* Merge banner shown after a Reconnect-with-conflicts — hidden only in demo mode */}
      {!demo && (
        <MergeBanner
          key={bannerKey}
          conflictCount={bannerCount}
          onDismiss={() => setBannerCount(0)}
        />
      )}

      {/* Split-screen device panels */}
      <div className="grid gap-4 md:grid-cols-2">
        <DevicePanel
          deviceId="device-a"
          label="Device A"
          showTimestamps={showTimestamps}
          onMergeDetected={handleMergeDetected}
        />
        <DevicePanel
          deviceId="device-b"
          label="Device B"
          showTimestamps={showTimestamps}
          onMergeDetected={handleMergeDetected}
        />
      </div>
    </div>
  );
}
