/**
 * Banner encouraging users to open the app in another tab
 * to see real-time sync in action.
 */
export function QRBanner() {
  const url = typeof window !== 'undefined' ? window.location.href : '';

  return (
    <div className="flex items-center justify-between rounded-lg border border-border bg-surface px-4 py-3">
      <div>
        <p className="text-sm font-medium text-text">
          Open in another tab to see real-time sync
        </p>
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-xs text-primary hover:text-primary-dark transition-colors"
        >
          {url}
        </a>
      </div>
      <a
        href={url}
        target="_blank"
        rel="noopener noreferrer"
        className="shrink-0 rounded-lg border border-border bg-surface-light px-3 py-1.5 text-sm font-medium text-text hover:bg-surface-lighter transition-colors"
      >
        Open new tab
      </a>
    </div>
  );
}
