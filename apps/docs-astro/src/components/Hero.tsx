import React, { useState } from 'react';
import { ArrowRight, Terminal, Copy, Check } from 'lucide-react';
import { useShiki } from '../hooks/useShiki';

const HERO_CODE = `import { TopGun } from '@topgunbuild/client';

// 1. Initialize Local-First DB
const db = new TopGun({
  sync: 'wss://api.topgun.dev',
  persist: 'indexeddb'
});

// 2. Zero-Latency Write (Optimistic)
await db.todos.set({
  id: 'task-1',
  text: 'Ship v2',
  status: 'pending'
}); // Resolves in ~0.5ms`;

const CodeSnippet = () => {
  const [copied, setCopied] = useState(false);
  const { html: highlightedCode } = useShiki(HERO_CODE, 'typescript');

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(HERO_CODE);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      console.error('Failed to copy to clipboard');
    }
  };

  return (
    <div className="w-full max-w-lg mx-auto lg:mx-0 rounded-xl overflow-hidden border border-white/10 bg-[#0d0d0d] shadow-2xl shadow-blue-900/10">
      <div className="flex items-center justify-between px-4 py-3 bg-white/5 border-b border-white/5">
        <div className="flex space-x-2">
          <div className="w-3 h-3 rounded-full bg-red-500/20 border border-red-500/50"></div>
          <div className="w-3 h-3 rounded-full bg-yellow-500/20 border border-yellow-500/50"></div>
          <div className="w-3 h-3 rounded-full bg-green-500/20 border border-green-500/50"></div>
        </div>
        <div className="flex items-center gap-2">
           <span className="text-xs text-neutral-500 font-mono">client.ts</span>
           <button onClick={handleCopy} className="text-neutral-500 hover:text-white transition-colors">
             {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
           </button>
        </div>
      </div>
      <div className="p-4 overflow-x-auto">
        {highlightedCode ? (
          <div 
            dangerouslySetInnerHTML={{ __html: highlightedCode }} 
            className="text-sm font-mono leading-relaxed [&_pre]:!bg-transparent [&_pre]:!p-0 [&_pre]:!m-0 [&_code]:!bg-transparent"
          />
        ) : (
          <pre className="font-mono text-sm leading-relaxed text-neutral-300">
            <code>{HERO_CODE}</code>
          </pre>
        )}
      </div>
    </div>
  );
};

export const Hero = () => {
  return (
    <section className="relative pt-32 pb-20 lg:pt-48 lg:pb-32 px-4 sm:px-6 lg:px-8 max-w-7xl mx-auto">
       <div className="flex flex-col lg:flex-row items-center gap-16">
         {/* Text Content */}
         <div className="flex-1 text-center lg:text-left">
            <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-blue-500/10 border border-blue-500/20 text-blue-600 dark:text-blue-400 text-sm font-medium mb-8">
              <span className="flex h-2 w-2 rounded-full bg-blue-600 dark:bg-blue-400 animate-pulse"></span>
              The Hybrid Offline-First In-Memory Data Grid
            </div>
            
            <h1 className="text-5xl sm:text-6xl lg:text-7xl font-bold tracking-tight text-foreground mb-6">
              Invert Your <br/>
              <span className="text-transparent bg-clip-text bg-gradient-to-r from-blue-600 via-purple-600 to-black dark:from-blue-400 dark:via-purple-400 dark:to-white animate-pulse-slow">Data Architecture.</span>
            </h1>
            
            <p className="text-lg text-neutral-600 dark:text-neutral-300 mb-8 max-w-2xl mx-auto lg:mx-0 leading-relaxed">
              Stop building "dumb" clients. TopGun turns your client into a replica. 
              Zero-latency reads/writes, offline-first reliability, and real-time sync 
              powered by CRDTs and Merkle Trees.
            </p>

            <div className="flex flex-col sm:flex-row items-center gap-4 justify-center lg:justify-start">
              <a
                href="/docs/intro"
                className="h-12 px-8 rounded-lg bg-foreground text-background font-semibold hover:opacity-90 transition-opacity flex items-center gap-2"
              >
                Get Started
                <ArrowRight className="w-4 h-4" />
              </a>
              <a
                href="/whitepaper"
                className="h-12 px-8 rounded-lg bg-black/5 dark:bg-white/5 border border-black/10 dark:border-white/10 text-foreground font-medium hover:bg-black/10 dark:hover:bg-white/10 transition-colors flex items-center"
              >
                Read Whitepaper
              </a>
            </div>
            
            <div className="mt-10 flex items-center justify-center lg:justify-start gap-4 text-sm text-neutral-500 dark:text-neutral-300 font-mono">
              <span className="flex items-center gap-2">
                <Terminal className="w-4 h-4" /> npm install @topgunbuild/client
              </span>
            </div>
         </div>

         {/* Visual/Code */}
         <div className="flex-1 w-full max-w-[600px] lg:max-w-none perspective-1000">
            <div className="relative transform lg:rotate-y-[-5deg] lg:rotate-x-[5deg] transition-transform duration-500 hover:rotate-0">
               {/* Decorative Glow */}
               <div className="absolute -inset-1 bg-gradient-to-r from-blue-600 to-purple-600 rounded-xl blur opacity-20"></div>
               <CodeSnippet />
               
               {/* Floating Badge */}
               <div className="absolute -bottom-6 -right-6 hidden sm:flex items-center gap-3 px-4 py-3 bg-white dark:bg-[#111] border border-black/10 dark:border-white/10 rounded-lg shadow-xl">
                  <div className="flex flex-col">
                    <span className="text-[10px] uppercase text-neutral-500 font-semibold tracking-wider">Sync Latency</span>
                    <span className="text-green-600 dark:text-green-400 font-mono font-bold">~16ms</span>
                  </div>
                  <div className="h-8 w-[1px] bg-black/10 dark:bg-white/10"></div>
                  <div className="flex flex-col">
                    <span className="text-[10px] uppercase text-neutral-500 font-semibold tracking-wider">Consistency</span>
                    <span className="text-blue-600 dark:text-blue-400 font-bold">Strong Eventual</span>
                  </div>
               </div>
            </div>
         </div>
       </div>
    </section>
  );
};
