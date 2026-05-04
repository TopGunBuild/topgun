import React, { useState } from 'react';
import { Terminal, Copy, Check } from 'lucide-react';

const CLI_COMMAND = 'npx create-topgun-app';
const DROP_IN_CMD = 'pnpm start:server';
const DROP_IN_DOCKER_ALT = 'docker compose up server';
const PROD_LINK = '/docs/roadmap';

const QuickStartTabs = () => {
  const [tab, setTab] = useState<'drop-in' | 'production'>('drop-in');

  return (
    <div className="mt-2 mb-2">
      <div role="tablist" aria-label="Quick start path" className="inline-flex rounded-lg border border-white/10 bg-black/5 dark:bg-white/5 p-1 text-sm">
        <button
          role="tab"
          data-tab="drop-in"
          aria-selected={tab === 'drop-in'}
          onClick={() => setTab('drop-in')}
          className={`px-3 py-1 rounded-md transition-colors ${tab === 'drop-in' ? 'bg-foreground text-background' : 'text-neutral-500 hover:text-foreground'}`}
        >
          Drop-in
        </button>
        <button
          role="tab"
          data-tab="production"
          aria-selected={tab === 'production'}
          onClick={() => setTab('production')}
          className={`px-3 py-1 rounded-md transition-colors ${tab === 'production' ? 'bg-foreground text-background' : 'text-neutral-500 hover:text-foreground'}`}
        >
          Production
        </button>
      </div>
      <div className="mt-2 text-sm text-neutral-600 dark:text-neutral-300">
        {tab === 'drop-in' ? (
          <p>
            Single-node backend in a separate terminal: <code className="px-1.5 py-0.5 rounded bg-black/5 dark:bg-white/5 font-mono text-xs">{DROP_IN_CMD}</code>. Zero config — embedded storage at <code className="px-1.5 py-0.5 rounded bg-black/5 dark:bg-white/5 font-mono text-xs">./topgun.redb</code>, no Docker, no Postgres. (Docker alternative: <code className="px-1.5 py-0.5 rounded bg-black/5 dark:bg-white/5 font-mono text-xs">{DROP_IN_DOCKER_ALT}</code>.) No auth required — local exploration.
          </p>
        ) : (
          <>
            <p>
              Postgres + JWT + single-node deployment. Multi-node clustering is on the <a href={PROD_LINK} className="underline underline-offset-2 hover:text-foreground">roadmap (Raft consensus)</a>.
            </p>
            <p className="mt-1">
              <a href="/docs/intro#production" className="hover:text-foreground hover:underline underline-offset-2">See production checklist →</a>
            </p>
          </>
        )}
      </div>
    </div>
  );
};

const CommandBlock = () => {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(CLI_COMMAND);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Mirror CodeSnippet behavior: silently log, leave icon unchanged.
      console.error('Failed to copy to clipboard');
    }
  };

  return (
    <div className="mt-8 mb-2">
      <div className="flex items-center justify-between w-full rounded-lg border border-white/10 bg-black/5 dark:bg-white/5 px-4 py-3 font-mono text-sm sm:text-base">
        <div className="flex items-center gap-2 min-w-0">
          <Terminal className="w-4 h-4 text-neutral-500 shrink-0" />
          <span className="text-neutral-500 shrink-0">$</span>
          <span className="text-foreground truncate">{CLI_COMMAND}</span>
        </div>
        <button
          onClick={handleCopy}
          aria-label={copied ? 'Copied' : 'Copy command'}
          className="ml-3 shrink-0 text-neutral-500 hover:text-foreground transition-all active:scale-90 min-w-[40px] min-h-[40px] flex items-center justify-center"
        >
          {copied ? (
            <Check className="w-4 h-4 text-green-500 transition-opacity duration-200" />
          ) : (
            <Copy className="w-4 h-4 transition-opacity duration-200" />
          )}
        </button>
      </div>
      <p className="mt-2 text-sm text-neutral-500 dark:text-neutral-400 text-center lg:text-left">
        <a href="/docs/intro" className="hover:text-foreground transition-colors underline underline-offset-2">
          Or read the docs first →
        </a>
      </p>
    </div>
  );
};

export default function HeroQuickStart() {
  return (
    <>
      <CommandBlock />
      <QuickStartTabs />
    </>
  );
}
