import React from 'react';
import { Check, X, Minus } from 'lucide-react';

const ComparisonRow = ({ feature, tg, es, fb, rx }: { feature: string, tg: any, es: any, fb: any, rx: any }) => (
  <tr className="border-b border-card-border hover:bg-black/5 dark:hover:bg-white/[0.02] transition-colors">
    <td className="py-4 px-4 text-sm font-medium text-neutral-700 dark:text-neutral-300">{feature}</td>
    <td className="py-4 px-4 text-center bg-blue-500/5 border-x border-blue-500/10">
      <div className="flex justify-center text-blue-600 dark:text-blue-400 font-semibold text-sm">{tg}</div>
    </td>
    <td className="py-4 px-4 text-center text-neutral-600 dark:text-neutral-300 text-sm">{es}</td>
    <td className="py-4 px-4 text-center text-neutral-600 dark:text-neutral-300 text-sm">{fb}</td>
    <td className="py-4 px-4 text-center text-neutral-600 dark:text-neutral-300 text-sm">{rx}</td>
  </tr>
);

export const Comparison = () => {
  return (
    <section id="comparison" className="py-24 px-4 sm:px-6 lg:px-8 max-w-7xl mx-auto">
      <div className="text-center mb-16">
        <h2 className="text-3xl font-bold text-foreground mb-4">Why TopGun?</h2>
        <p className="text-neutral-600 dark:text-neutral-300">
          The only solution that combines the speed of an In-Memory Data Grid with robust Offline-First capabilities.
        </p>
      </div>

      <div className="overflow-x-auto">
        <table className="w-full text-left border-collapse">
          <thead>
            <tr className="border-b border-card-border">
              <th className="py-4 px-4 text-sm font-mono text-neutral-500 uppercase tracking-wider">Feature</th>
              <th className="py-4 px-4 text-center bg-blue-500/5 border-t border-x border-blue-500/10 rounded-t-lg">
                <span className="text-foreground font-bold text-lg">TopGun</span>
              </th>
              <th className="py-4 px-4 text-center text-neutral-500 font-medium">ElectricSQL</th>
              <th className="py-4 px-4 text-center text-neutral-500 font-medium">Firebase</th>
              <th className="py-4 px-4 text-center text-neutral-500 font-medium">RxDB</th>
            </tr>
          </thead>
          <tbody>
            <ComparisonRow
              feature="Primary Model"
              tg="Local-First IMDG"
              es="Postgres Sync"
              fb="Cloud Doc DB"
              rx="Local-First DB"
            />
            <ComparisonRow
              feature="Offline Support"
              tg={<span className="flex items-center gap-1"><Check className="w-4 h-4" /> First-Class</span>}
              es={<span className="flex items-center gap-1 justify-center"><Check className="w-4 h-4" /> Good</span>}
              fb="Good"
              rx={<span className="flex items-center gap-1 justify-center"><Check className="w-4 h-4" /> Excellent</span>}
            />
            <ComparisonRow
              feature="Latency"
              tg="~0ms (In-Memory)"
              es="~5-10ms (SQLite)"
              fb="Network Dependent"
              rx="~5-10ms (IndexedDB)"
            />
            <ComparisonRow
              feature="Backend Control"
              tg="Self-Hosted Cluster"
              es="Sync Service"
              fb={<span className="flex items-center gap-1 justify-center text-red-500 dark:text-red-400"><X className="w-4 h-4" /> Proprietary</span>}
              rx="CouchDB / Custom"
            />
            <ComparisonRow
              feature="Consistency"
              tg="HLC + CRDT"
              es="Rich CRDTs"
              fb="LWW (Server)"
              rx="Revision Trees"
            />
            <ComparisonRow
              feature="Distributed Locks"
              tg={<span className="flex items-center gap-1"><Check className="w-4 h-4" /> Fencing Tokens</span>}
              es={<span className="flex items-center gap-1 justify-center text-red-500 dark:text-red-400"><X className="w-4 h-4 " /> Not Supported</span>}
              fb={<span className="flex items-center gap-1 justify-center text-red-500 dark:text-red-400"><X className="w-4 h-4" /> Not Supported</span>}
              rx={<span className="flex items-center gap-1 justify-center text-red-500 dark:text-red-400"><X className="w-4 h-4" /> Not Supported</span>}
            />
            <ComparisonRow
              feature="License"
              tg="Open Source"
              es="Open Source"
              fb="Proprietary"
              rx="Open Source"
            />
          </tbody>
        </table>
      </div>
    </section>
  );
};