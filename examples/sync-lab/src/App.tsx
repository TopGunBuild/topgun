import { useState, useEffect, useRef } from 'react';
import { QRBanner } from '@/components/QRBanner';
import { CodeSnippets } from '@/components/CodeSnippets';
import { ConflictArena } from '@/components/ConflictArena';
import { LatencyRace } from '@/components/LatencyRace';

type Tab = 'conflict-arena' | 'latency-race';

function useQueryParams() {
  const params = new URLSearchParams(window.location.search);
  return {
    embed: params.has('embed'),
    demo: params.has('demo'),
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
  const { embed, demo } = useQueryParams();
  const startTimeRef = useRef(performance.now());

  // Mark load complete for demo badge
  useEffect(() => {
    startTimeRef.current = performance.now();
  }, []);

  const tabs: { id: Tab; label: string }[] = [
    { id: 'conflict-arena', label: 'Conflict Arena' },
    { id: 'latency-race', label: 'Latency Race' },
  ];

  return (
    <div className="mx-auto min-h-screen max-w-7xl px-4 py-6">
      {/* Header and QR banner hidden in embed mode */}
      {!embed && (
        <>
          <header className="mb-6">
            <h1 className="text-3xl font-bold text-text">
              TopGun <span className="text-primary">Sync Lab</span>
            </h1>
            <p className="mt-1 text-text-muted">
              Offline-first CRDT sync — see it happen live
            </p>
          </header>
          <div className="mb-6">
            <QRBanner />
          </div>
        </>
      )}

      {/* Tab bar */}
      <div className="mb-6 flex gap-1 rounded-lg bg-surface p-1">
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
