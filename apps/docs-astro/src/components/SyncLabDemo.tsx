import { useState, useEffect, useRef } from 'react';

const DEMO_URL = 'https://demo.topgun.build';

export function SyncLabDemo() {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [theme, setTheme] = useState<'dark' | 'light'>(() =>
    document.documentElement.classList.contains('dark') ? 'dark' : 'light'
  );

  // Watch for dark class changes on the host page
  useEffect(() => {
    const observer = new MutationObserver(() => {
      const isDark = document.documentElement.classList.contains('dark');
      setTheme(isDark ? 'dark' : 'light');
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    });
    return () => observer.disconnect();
  }, []);

  // Send theme changes to iframe via postMessage (avoids iframe reload)
  useEffect(() => {
    iframeRef.current?.contentWindow?.postMessage(
      { type: 'theme-change', theme },
      DEMO_URL,
    );
  }, [theme]);

  // Initial src includes theme; subsequent changes use postMessage
  const src = `${DEMO_URL}?embed&theme=${theme}`;
  const srcRef = useRef(src);

  return (
    <section
      id="demo"
      className="py-16 border-t border-card-border bg-neutral-50 dark:bg-background transition-colors"
    >
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        {/* Section Header */}
        <div className="text-center mb-10">
          <h2 className="text-3xl font-bold text-foreground mb-4">
            Live Sync Demo
          </h2>
          <p className="text-neutral-600 dark:text-neutral-300 max-w-2xl mx-auto">
            Two devices, one session — watch CRDT conflict resolution happen in
            real time. Open in a{' '}
            <a
              href={DEMO_URL}
              target="_blank"
              rel="noreferrer"
              className="text-blue-600 dark:text-blue-400 underline underline-offset-2 hover:text-blue-800 dark:hover:text-blue-300"
            >
              new tab
            </a>{' '}
            for multi-tab sync.
          </p>
        </div>

        {/* Iframe Container */}
        <div className="rounded-2xl overflow-hidden border border-card-border shadow-2xl">
          <iframe
            ref={iframeRef}
            src={srcRef.current}
            title="TopGun Sync Lab"
            className="w-full border-0"
            style={{ height: '680px' }}
            allow="clipboard-write"
            loading="lazy"
          />
        </div>
      </div>
    </section>
  );
}
