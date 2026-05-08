import { useEffect } from 'react';

interface MergeBannerProps {
  conflictCount: number;
  onDismiss: () => void;
}

/**
 * Displays a success banner after a CRDT merge with conflicts, naming what
 * just happened (HLC auto-merge) so first-time visitors understand the result.
 * Auto-dismisses after 8s so it stays out of the way without requiring action.
 */
export function MergeBanner({ conflictCount, onDismiss }: MergeBannerProps) {
  useEffect(() => {
    if (conflictCount <= 0) return;
    const timer = setTimeout(onDismiss, 8000);
    return () => clearTimeout(timer);
  }, [conflictCount, onDismiss]);

  if (conflictCount <= 0) return null;

  const fieldLabel = conflictCount === 1 ? 'field' : 'fields';

  return (
    <div className="animate-fade-in mb-4 flex items-center justify-between rounded-lg border border-success/30 bg-success/10 px-4 py-3 text-text">
      <span className="text-sm font-medium">
        ✓ {conflictCount} {fieldLabel} auto-merged by HLC timestamps. No conflict-resolution code required.
      </span>
      <button
        onClick={onDismiss}
        aria-label="Dismiss merge banner"
        className="ml-4 shrink-0 text-text-muted hover:text-text transition-colors"
      >
        ×
      </button>
    </div>
  );
}
