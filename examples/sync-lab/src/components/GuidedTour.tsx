import { useState, useEffect } from 'react';

const STORAGE_KEY = 'topgun-sync-lab-tour-dismissed-v1';

const STEPS = [
  { n: 1, text: <>Click <strong>Disconnect</strong> on Device A</> },
  { n: 2, text: <>Edit the same todo on both devices</> },
  { n: 3, text: <>Click <strong>Reconnect</strong> on Device A</> },
  { n: 4, text: <>Watch the conflicts merge automatically</> },
];

/**
 * Four-step guided tour overlay shown on first visit to the Conflict Arena.
 * Reads its own dismissal state from localStorage so it self-manages visibility.
 * Caller gates rendering on !embed && !demo — this component only handles the
 * localStorage check so subsequent renders are instant (no flash on page reload).
 */
export function GuidedTour() {
  // Lazy initializer reads localStorage once on mount; avoids one-frame flash
  const [dismissed, setDismissed] = useState<boolean>(
    () => localStorage.getItem(STORAGE_KEY) === '1'
  );

  const dismiss = () => {
    localStorage.setItem(STORAGE_KEY, '1');
    setDismissed(true);
  };

  // Keyboard Esc dismissal for parity with backdrop click
  useEffect(() => {
    if (dismissed) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') dismiss();
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [dismissed]);

  if (dismissed) return null;

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/40"
      onClick={dismiss}
      aria-modal="true"
      role="dialog"
      aria-label="Guided tour"
    >
      {/* Stop click propagation so clicking the card does not dismiss */}
      <div
        className="relative w-full max-w-sm rounded-xl border border-border bg-bg p-6 shadow-xl mx-4"
        onClick={e => e.stopPropagation()}
      >
        <h2 className="mb-1 text-lg font-semibold text-text">How it works</h2>
        <p className="mb-4 text-sm text-text-muted">
          Follow these steps to trigger an automatic CRDT conflict merge:
        </p>

        <ol className="flex flex-col gap-3">
          {STEPS.map(step => (
            <li key={step.n} className="flex items-start gap-3">
              <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-sm font-bold text-primary">
                {step.n}
              </span>
              <span className="pt-0.5 text-sm text-text">{step.text}</span>
            </li>
          ))}
        </ol>

        <button
          onClick={dismiss}
          className="mt-6 w-full rounded-lg bg-primary px-4 py-2.5 text-sm font-semibold text-white hover:opacity-90 transition-opacity"
        >
          Got it
        </button>
      </div>
    </div>
  );
}
