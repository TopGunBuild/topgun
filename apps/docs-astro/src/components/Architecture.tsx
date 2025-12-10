import React from 'react';
import { Server, Database, Smartphone, Laptop, Cloud, ArrowLeftRight } from 'lucide-react';

const Node = ({ icon: Icon, label, sub }: { icon: any, label: string, sub?: string }) => (
  <div className="flex flex-col items-center justify-center p-4 rounded-xl border border-card-border bg-white/50 dark:bg-white/5 backdrop-blur-sm z-10 w-32 h-32 text-center transition-all hover:border-blue-500/50 hover:shadow-[0_0_30px_-10px_rgba(59,130,246,0.3)] shadow-sm dark:shadow-none">
    <Icon className="w-8 h-8 text-neutral-600 dark:text-neutral-300 mb-2" />
    <span className="text-sm font-semibold text-foreground">{label}</span>
    {sub && <span className="text-[10px] text-neutral-500 mt-1">{sub}</span>}
  </div>
);

const Connection = ({ active = false, reverse = false }: { active?: boolean, reverse?: boolean }) => (
  <div className="h-[2px] w-12 md:w-24 bg-black/10 dark:bg-white/10 relative overflow-hidden">
    {active && (
      <div
        className={`absolute inset-0 bg-gradient-to-r from-transparent via-blue-500 to-transparent w-1/2 ${reverse ? 'animate-[sync-pulse_2s_ease-in-out_infinite_reverse]' : 'animate-[sync-pulse_2s_ease-in-out_infinite]'}`}
      />
    )}
  </div>
);

export const Architecture = () => {
  return (
    <section id="architecture" className="py-24 border-t border-card-border bg-neutral-50 dark:bg-background transition-colors">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div className="text-center mb-16">
          <h2 className="text-3xl font-bold text-foreground mb-4">Architecture</h2>
          <p className="text-neutral-600 dark:text-neutral-300 max-w-2xl mx-auto">
            TopGun inverts the traditional model. The client is the primary source of truth for the UI, 
            while the server ensures eventual consistency and persistence.
          </p>
        </div>

        <div className="relative p-8 md:p-12 rounded-3xl border border-card-border bg-grid overflow-hidden bg-white dark:bg-card">
          <div className="absolute inset-0 bg-gradient-to-b from-transparent via-white/50 to-white dark:via-card/50 dark:to-card pointer-events-none"></div>
          
          <div className="relative flex flex-col md:flex-row items-center justify-center gap-4 md:gap-0">
            
            {/* Client Side */}
            <div className="flex flex-col gap-6 p-6 rounded-2xl border border-dashed border-neutral-300 dark:border-white/10 bg-neutral-100/50 dark:bg-white/[0.02]">
              <div className="text-xs font-mono text-neutral-500 uppercase tracking-widest text-center mb-2">Client Device</div>
              <div className="flex gap-4">
                <Node icon={Smartphone} label="Mobile" sub="SQLite" />
                <Node icon={Laptop} label="Browser" sub="IndexedDB" />
              </div>
              <div className="flex justify-center">
                 <div className="px-4 py-2 rounded-full bg-blue-500/10 border border-blue-500/20 text-blue-600 dark:text-blue-400 text-xs font-mono">
                    Local CRDT Store
                 </div>
              </div>
            </div>

            {/* Connection */}
            <div className="flex md:flex-col items-center gap-2 px-4">
               <ArrowLeftRight className="w-5 h-5 text-neutral-600 mb-2 hidden md:block" />
               <Connection active />
               <div className="bg-white dark:bg-card border border-card-border px-3 py-1 rounded-md text-[10px] text-neutral-500 dark:text-neutral-300 font-mono whitespace-nowrap z-20 shadow-sm">
                 Merkle Sync (WS)
               </div>
               <Connection active reverse />
            </div>

            {/* Server Side */}
            <div className="flex flex-col gap-6 p-6 rounded-2xl border border-dashed border-neutral-300 dark:border-white/10 bg-neutral-100/50 dark:bg-white/[0.02]">
              <div className="text-xs font-mono text-neutral-500 uppercase tracking-widest text-center mb-2">Server Cluster</div>
              <div className="flex flex-col items-center gap-4">
                <Node icon={Server} label="TopGun Gateway" sub="Partition Engine" />
                <div className="h-8 w-[2px] bg-black/10 dark:bg-white/10"></div>
                <Node icon={Database} label="Persistence" sub="Postgres / Mongo" />
              </div>
            </div>

          </div>
        </div>
      </div>
    </section>
  );
};