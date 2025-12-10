import React, { useState } from 'react';
import { Copy, Check } from 'lucide-react';
import { useShiki } from '../hooks/useShiki';

const HERO_CODE = `import { TopGun } from '@topgunbuild/client';

// 1. Initialize Local-First DB
const db = new TopGun({
  sync: 'wss://api.topgun.build',
  persist: 'indexeddb'
});

// 2. Zero-Latency Write (Optimistic)
await db.todos.set({
  id: 'task-1',
  text: 'Ship v2',
  status: 'pending'
}); // Resolves in ~0.5ms`;

export const HeroCodeSnippet = () => {
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
