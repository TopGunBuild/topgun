import React from 'react';
import { WifiOff, Zap, RefreshCw, Database, Lock, Globe, Shield } from 'lucide-react';

const FeatureCard = ({ icon: Icon, title, description, colSpan = 1 }: { icon: any, title: string, description: string, colSpan?: number }) => (
  <div className={`group relative overflow-hidden rounded-xl border border-card-border bg-card p-8 hover:border-neutral-300 dark:hover:border-white/20 transition-colors shadow-sm dark:shadow-none ${colSpan === 2 ? 'md:col-span-2' : ''}`}>
    <div className="absolute top-0 right-0 -mt-4 -mr-4 w-24 h-24 bg-gradient-to-br from-blue-500/10 to-transparent rounded-full blur-2xl group-hover:from-blue-500/20 transition-all duration-500"></div>

    <div className="relative z-10">
      <div className="w-12 h-12 rounded-lg bg-black/5 dark:bg-black/50 border border-black/10 dark:border-white/10 flex items-center justify-center mb-6 text-foreground group-hover:scale-110 transition-transform duration-300">
        <Icon className="w-6 h-6 text-blue-600 dark:text-white" />
      </div>
      <h3 className="text-xl font-bold text-foreground mb-3">{title}</h3>
      <p className="text-neutral-600 dark:text-neutral-300 leading-relaxed">
        {description}
      </p>
    </div>
  </div>
);

export const Features = () => {
  return (
    <section id="features" className="py-24 px-4 sm:px-6 lg:px-8 max-w-7xl mx-auto relative">
      <div className="mb-16">
        <h2 className="text-3xl md:text-5xl font-bold text-foreground mb-6">
          The <span className="text-transparent bg-clip-text bg-gradient-to-r from-blue-600 to-purple-600 dark:from-blue-400 dark:to-purple-400">Local-First</span> Standard.
        </h2>
        <p className="text-xl text-neutral-600 dark:text-neutral-300 max-w-3xl">
          TopGun bridges the gap between scalable In-Memory Data Grids and Offline-First Client Databases.
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <FeatureCard
          colSpan={2}
          icon={Zap}
          title="Zero-Latency UI"
          description="Reads and writes happen locally against an in-memory CRDT. The UI updates in <16ms (one frame) using Optimistic UI patterns by default. No spinners, no waiting."
        />
        <FeatureCard
          icon={WifiOff}
          title="Offline Capable"
          description="The network is optional. Data is automatically persisted to IndexedDB (Browser) or SQLite (Mobile/Desktop) and synced when connectivity returns."
        />
        <FeatureCard
          icon={RefreshCw}
          title="Merkle Tree Sync"
          description="Efficient bandwidth usage. We only exchange the modified leaves of the Merkle Tree, drastically reducing data transfer costs compared to REST/GraphQL."
        />
        <FeatureCard
          colSpan={2}
          icon={Database}
          title="Scalable Backend"
          description="A server-authoritative cluster that coordinates HLC (Hybrid Logical Clocks) and persists to standard databases like PostgreSQL or Mongo. You own your data."
        />
        <FeatureCard
          icon={Lock}
          title="Conflict Resolution"
          description="Automatic convergence via CRDTs. No manual conflict resolution code required for standard operations."
        />
        <FeatureCard
          icon={Shield}
          title="Distributed Locks"
          description="Pessimistic locking with fencing tokens prevents split-brain scenarios. Essential for critical operations like payments or inventory updates."
        />
        <FeatureCard
          icon={Globe}
          title="Real-Time"
          description="Push-based architecture via WebSockets ensures all active clients receive updates instantly."
        />
      </div>
    </section>
  );
};