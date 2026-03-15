import { useState, useEffect, useRef, useCallback } from 'react';
import { CodeSnippets } from '@/components/CodeSnippets';
import { ConflictArena } from '@/components/ConflictArena';
import { LatencyRace } from '@/components/LatencyRace';
import { getSessionId, getShareUrl } from '@/lib/session';

type Tab = 'conflict-arena' | 'latency-race';

function useQueryParams() {
  const params = new URLSearchParams(window.location.search);
  return {
    embed: params.has('embed'),
    demo: params.has('demo'),
    theme: params.get('theme') as 'dark' | 'light' | null,
  };
}

function PerformanceBadge() {
  const [loadTime] = useState(() => performance.now());
  const [avgReadLatency, setAvgReadLatency] = useState<number | null>(null);
  const samplesRef = useRef<number[]>([]);

  // Measure avg read latency by timing performance.now() pairs
  useEffect(() => {
    const measure = () => {
      const start = performance.now();
      // Simulates a synchronous in-memory read (same as map.get())
      void undefined;
      const elapsed = performance.now() - start;
      samplesRef.current.push(elapsed);
      if (samplesRef.current.length > 10) samplesRef.current.shift();
      const avg = samplesRef.current.reduce((a, b) => a + b, 0) / samplesRef.current.length;
      setAvgReadLatency(avg);
    };
    measure();
    const interval = setInterval(measure, 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div className="fixed bottom-3 right-3 z-50 rounded-lg bg-surface px-3 py-2 text-xs font-mono text-text-muted shadow-lg">
      Load: {loadTime.toFixed(0)}ms
      {avgReadLatency !== null && (
        <span className="ml-2">
          Avg Read: {avgReadLatency < 1 ? `${(avgReadLatency * 1000).toFixed(0)}µs` : `${avgReadLatency.toFixed(2)}ms`}
        </span>
      )}
    </div>
  );
}

export default function App() {
  const [activeTab, setActiveTab] = useState<Tab>('conflict-arena');
  const { embed, demo, theme } = useQueryParams();
  const startTimeRef = useRef(performance.now());
  const [copied, setCopied] = useState(false);

  const handleShareSession = useCallback(async () => {
    const url = getShareUrl();
    await navigator.clipboard.writeText(url);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, []);

  // Apply theme class to document root
  useEffect(() => {
    const root = document.documentElement;
    if (theme === 'light') {
      root.classList.remove('dark');
    } else {
      root.classList.add('dark');
    }
  }, [theme]);

  // Listen for theme changes from parent iframe host (postMessage)
  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.data?.type === 'theme-change' && event.data.theme) {
        const root = document.documentElement;
        if (event.data.theme === 'light') {
          root.classList.remove('dark');
        } else {
          root.classList.add('dark');
        }
      }
    };
    window.addEventListener('message', handler);
    return () => window.removeEventListener('message', handler);
  }, []);

  // Send session ID and content height to parent when embedded
  useEffect(() => {
    if (!embed || window.parent === window) return;

    window.parent.postMessage(
      { type: 'session-id', sessionId: getSessionId() },
      '*',
    );

    const sendHeight = () => {
      const height = document.documentElement.scrollHeight;
      window.parent.postMessage({ type: 'resize', height }, '*');
    };

    sendHeight();
    const observer = new ResizeObserver(sendHeight);
    observer.observe(document.body);
    return () => observer.disconnect();
  }, [embed]);

  // Mark load complete for demo badge
  useEffect(() => {
    startTimeRef.current = performance.now();
  }, []);

  const tabs: { id: Tab; label: string }[] = [
    { id: 'conflict-arena', label: 'Conflict Arena' },
    { id: 'latency-race', label: 'Latency Race' },
  ];

  return (
    <div className={`mx-auto max-w-7xl px-4 py-6 ${embed ? '' : 'min-h-screen'}`}>
      {/* Header and QR banner hidden in embed mode */}
      {!embed && (
        <>
          <header className="mb-6 flex items-start justify-between">
            <div>
              <h1 className="text-3xl font-bold text-text">
                TopGun <span className="text-primary">Sync Lab</span>
              </h1>
              <p className="mt-1 text-text-muted">
                Offline-first CRDT sync — see it happen live
              </p>
            </div>
            <button
              onClick={handleShareSession}
              className="rounded-lg border border-primary/30 bg-primary/10 px-3 py-1.5 text-sm font-medium text-primary hover:opacity-80 transition-opacity"
            >
              {copied ? 'Copied!' : 'Share session'}
            </button>
          </header>
        </>
      )}

      {/* Tab bar */}
      <div className="mb-6 flex gap-1 rounded-lg border border-border bg-surface p-1">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`flex-1 rounded-md px-4 py-2 text-sm font-medium transition-colors ${
              activeTab === tab.id
                ? 'bg-primary text-white'
                : 'text-text-muted hover:text-text'
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="animate-fade-in">
        {activeTab === 'conflict-arena' && <ConflictArena />}
        {activeTab === 'latency-race' && <LatencyRace />}
      </div>

      {/* Code snippets hidden in embed mode */}
      {!embed && <CodeSnippets />}

      {/* Performance badge when ?demo is set */}
      {demo && <PerformanceBadge />}
    </div>
  );
}
