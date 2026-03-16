import type { LogEntry } from '@/hooks/useStateLog';

interface StateLogProps {
  entries: LogEntry[];
  visible: boolean;
  onToggle: () => void;
  onClear: () => void;
}

function entryColor(type: LogEntry['type']): string {
  switch (type) {
    case 'local-write':
      return 'text-primary';
    case 'remote-merge':
      return 'text-warning';
    case 'sync':
      return 'text-success';
    default:
      return 'text-text-muted';
  }
}

function entryLabel(type: LogEntry['type']): string {
  switch (type) {
    case 'local-write':
      return '[Local Write]';
    case 'remote-merge':
      return '[Remote Merge]';
    case 'sync':
      return '[Sync]';
    default:
      return '[Unknown]';
  }
}

/**
 * "Show State/Network" toggle panel with a running event log.
 * Displays local writes, remote merges, and connection state changes
 * with HLC timestamps. Capped at 100 entries.
 */
export function StateLog({ entries, visible, onToggle, onClear }: StateLogProps) {
  return (
    <div className="mt-4">
      <div className="flex items-center gap-3">
        <button
          onClick={onToggle}
          className="rounded-md border border-border bg-bg px-3 py-1.5 text-sm text-text-muted hover:text-text transition-colors"
        >
          {visible ? 'Hide' : 'Show'} State / Network
        </button>
        {visible && entries.length > 0 && (
          <button
            onClick={onClear}
            className="text-xs text-text-muted hover:text-danger transition-colors"
          >
            Clear
          </button>
        )}
      </div>

      {visible && (
        <div className="mt-2 max-h-64 overflow-y-auto rounded-lg border border-border bg-bg p-3 font-mono text-xs">
          {entries.length === 0 ? (
            <span className="text-text-muted">No events yet. Interact with the demo above.</span>
          ) : (
            entries.map(entry => (
              <div
                key={entry.id}
                className="animate-slide-in border-b border-surface-light py-1.5 last:border-0"
              >
                <span className={entryColor(entry.type)}>{entryLabel(entry.type)}</span>{' '}
                {entry.key && (
                  <span className="text-text">
                    {entry.key} = {JSON.stringify(entry.value)}
                  </span>
                )}
                {entry.message && <span className="text-text">{entry.message}</span>}
                {entry.hlc && (
                  <span className="ml-2 text-text-muted">HLC: {entry.hlc}</span>
                )}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
