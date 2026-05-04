import React from 'react';

export const ComingFromTable = () => {
  return (
    <section id="coming-from" className="py-24 px-4 sm:px-6 lg:px-8 max-w-7xl mx-auto">
      <div className="text-center mb-16">
        <h2 className="text-3xl font-bold text-foreground mb-4">Coming from X</h2>
        <p className="text-neutral-600 dark:text-neutral-300">
          Migrating from another sync stack? Each guide names what TopGun does not replace, up front.
        </p>
      </div>

      <div className="overflow-x-auto">
        <table className="w-full text-left border-collapse">
          <thead>
            <tr className="border-b border-card-border">
              <th className="py-3 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider">Coming from</th>
              <th className="py-3 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider">Why migrate</th>
              <th className="py-3 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider">Migration guide</th>
              <th className="py-3 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider">Status</th>
            </tr>
          </thead>
          <tbody>
            <tr className="border-b border-card-border">
              <td className="py-3 px-4 text-sm text-foreground">Firebase Realtime</td>
              <td className="py-3 px-4 text-sm text-foreground">SQL, FTS, no vendor lock-in</td>
              <td className="py-3 px-4 text-sm text-foreground">
                <a href="/docs/guides/migrating-from-firebase" className="text-brand hover:underline">Guide</a>
              </td>
              <td className="py-3 px-4 text-sm text-foreground">Live</td>
            </tr>
            <tr className="border-b border-card-border">
              <td className="py-3 px-4 text-sm text-foreground">Y.js / Automerge</td>
              <td className="py-3 px-4 text-sm text-foreground">Server backend + SQL queries</td>
              <td className="py-3 px-4 text-sm text-foreground">
                <a href="/docs/guides/migrating-from-yjs" className="text-brand hover:underline">Guide</a>
              </td>
              <td className="py-3 px-4 text-sm text-foreground">Live</td>
            </tr>
            <tr className="border-b border-card-border">
              <td className="py-3 px-4 text-sm text-foreground">Replicache</td>
              <td className="py-3 px-4 text-sm text-foreground">Open source, no SaaS invoice</td>
              <td className="py-3 px-4 text-sm text-foreground">
                <a href="/docs/guides/migrating-from-replicache" className="text-brand hover:underline">Guide</a>
              </td>
              <td className="py-3 px-4 text-sm text-foreground">Live</td>
            </tr>
            <tr className="border-b border-card-border">
              <td className="py-3 px-4 text-sm text-foreground">Supabase Realtime</td>
              <td className="py-3 px-4 text-sm text-foreground">Offline-first, CRDT auto-merge</td>
              <td className="py-3 px-4 text-sm text-foreground">
                <a href="/docs/guides/migrating-from-supabase-realtime" className="text-brand hover:underline">Guide</a>
              </td>
              <td className="py-3 px-4 text-sm text-foreground">Live</td>
            </tr>
            <tr className="border-b border-card-border">
              <td className="py-3 px-4 text-sm text-foreground">Liveblocks</td>
              <td className="py-3 px-4 text-sm text-foreground">Self-hosted option</td>
              <td className="py-3 px-4 text-sm text-neutral-500">(planned)</td>
              <td className="py-3 px-4 text-sm text-neutral-500">Q3 2026</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  );
};
