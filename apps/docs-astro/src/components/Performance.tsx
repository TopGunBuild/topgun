import React from 'react';

const REPO_BLOB = 'https://github.com/TopGunBuild/topgun/blob/main';

export const Performance = () => {
  return (
    <section id="performance" className="py-24 px-4 sm:px-6 lg:px-8 max-w-7xl mx-auto">
      <div className="text-center mb-12">
        <h2 className="text-3xl font-bold text-foreground mb-4">Performance</h2>
        <p className="text-neutral-600 dark:text-neutral-300 max-w-2xl mx-auto">
          Measured on Apple M1 Max, 200 concurrent WebSocket connections, using the in-process load
          harness in <code className="font-mono text-sm">packages/server-rust/benches/load_harness/</code>.
        </p>
      </div>

      <div className="overflow-x-auto max-w-3xl mx-auto">
        <table className="w-full text-left border-collapse">
          <thead>
            <tr className="border-b border-card-border">
              <th className="py-4 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider">Mode</th>
              <th className="py-4 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider text-right">Throughput</th>
              <th className="py-4 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider text-right">Latency</th>
            </tr>
          </thead>
          <tbody>
            <tr className="border-b border-card-border hover:bg-black/5 dark:hover:bg-white/[0.02] transition-colors">
              <td className="py-4 px-4 text-sm font-medium text-neutral-700 dark:text-neutral-300">Fire-and-forget</td>
              <td className="py-4 px-4 text-right text-brand font-semibold text-sm">483K ops/sec</td>
              <td className="py-4 px-4 text-right text-neutral-500 text-sm">—</td>
            </tr>
            <tr className="border-b border-card-border hover:bg-black/5 dark:hover:bg-white/[0.02] transition-colors">
              <td className="py-4 px-4 text-sm font-medium text-neutral-700 dark:text-neutral-300">Fire-and-wait</td>
              <td className="py-4 px-4 text-right text-brand font-semibold text-sm">~37K ops/sec</td>
              <td className="py-4 px-4 text-right text-brand font-semibold text-sm">1.5ms p50</td>
            </tr>
          </tbody>
        </table>
      </div>

      <p className="mt-8 max-w-3xl mx-auto text-sm text-neutral-500 dark:text-neutral-400 border-l-2 border-card-border pl-4">
        Numbers are from an in-process load harness on Apple M1 Max with 200 concurrent connections.
        Performance on your hardware will differ. See{' '}
        <a
          href={`${REPO_BLOB}/packages/server-rust/benches/load_harness/baseline.json`}
          target="_blank"
          rel="noopener noreferrer"
          className="text-brand hover:underline font-mono"
        >
          baseline.json
        </a>{' '}
        for CI thresholds and{' '}
        <a
          href={`${REPO_BLOB}/packages/server-rust/docs/profiling/FLAMEGRAPH_ANALYSIS.md`}
          target="_blank"
          rel="noopener noreferrer"
          className="text-brand hover:underline font-mono"
        >
          FLAMEGRAPH_ANALYSIS.md
        </a>{' '}
        for methodology.
      </p>
    </section>
  );
};
