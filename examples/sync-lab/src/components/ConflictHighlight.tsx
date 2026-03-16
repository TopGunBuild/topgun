import { useEffect, useState } from 'react';
import type { FieldConflict } from '@/lib/conflict-detector';
import { formatTimestamp } from '@/lib/conflict-detector';

interface ConflictHighlightProps {
  conflicts: FieldConflict[];
  showTimestamps: boolean;
}

/**
 * Renders per-field conflict indicators after a reconnect-merge.
 * Green = values matched (no conflict), Yellow = LWW resolved a conflict.
 * Highlights fade after 3 seconds via CSS animation.
 */
export function ConflictHighlight({
  conflicts,
  showTimestamps,
}: ConflictHighlightProps) {
  const [visible, setVisible] = useState(true);

  // Auto-hide after 3 seconds (the CSS animation also fades, but we
  // remove the DOM element to keep things tidy)
  useEffect(() => {
    const timer = setTimeout(() => setVisible(false), 3500);
    return () => clearTimeout(timer);
  }, []);

  if (!visible || conflicts.length === 0) return null;

  // Only show user-facing fields (skip _exists)
  const displayConflicts = conflicts.filter(c => c.field !== '_exists');

  return (
    <div className="mt-1 space-y-0.5">
      {displayConflicts.map(conflict => (
        <div
          key={conflict.field}
          className={`rounded px-2 py-0.5 text-xs ${
            conflict.status === 'matched'
              ? 'animate-highlight-green'
              : 'animate-highlight-yellow'
          }`}
        >
          <span className="font-medium">
            {conflict.field}:{' '}
          </span>
          {conflict.status === 'matched' ? (
            <span className="text-success">matched</span>
          ) : (
            <span className="text-warning">
              LWW resolved
              {showTimestamps && (
                <span className="ml-1 font-mono text-text-muted">
                  {formatTimestamp(conflict.winningTimestamp)}
                </span>
              )}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}
