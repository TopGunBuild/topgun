import { useState, useEffect } from 'react';

const SESSION_KEY = 'topgun-sync-lab-persistence-hint-shown';
const EVENT_NAME = 'topgun-sync-lab:first-todo-added';

/**
 * Shows a one-shot toast after the first todo is added in the session,
 * revealing that todos persist via IndexedDB even after a page refresh.
 * Uses a CustomEvent from DevicePanel so we avoid prop-drilling through
 * ConflictArena — this is a one-shot notification, not ongoing state.
 */
export function PersistenceToast(): JSX.Element | null {
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const handleFirstAdd = () => {
      // Defensive check — dispatcher also gates, but guard here in case
      // dispatcher gating ever regresses (e.g. in a future refactor)
      if (sessionStorage.getItem(SESSION_KEY) === '1') return;
      sessionStorage.setItem(SESSION_KEY, '1');
      setVisible(true);
    };

    window.addEventListener(EVENT_NAME, handleFirstAdd);
    return () => window.removeEventListener(EVENT_NAME, handleFirstAdd);
  }, []);

  useEffect(() => {
    if (!visible) return;
    // Auto-dismiss after 6 seconds
    const timer = setTimeout(() => setVisible(false), 6000);
    return () => clearTimeout(timer);
  }, [visible]);

  if (!visible) return null;

  return (
    <div
      className="animate-fade-in fixed top-4 right-4 z-50 max-w-xs cursor-pointer rounded-lg border border-primary/30 bg-bg px-4 py-3 shadow-lg text-sm text-text"
      onClick={() => setVisible(false)}
      role="status"
      aria-live="polite"
    >
      ↻ Refresh the page — your todos persist via IndexedDB.
    </div>
  );
}
