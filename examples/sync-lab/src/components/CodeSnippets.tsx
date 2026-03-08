import { Highlight, themes } from 'prism-react-renderer';

const snippets = [
  {
    title: 'Create a client and map',
    code: `import { TopGunClient } from '@topgunbuild/client';

const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  storage: new MemoryStorageAdapter(),
});

const todos = client.getMap('todos');`,
  },
  {
    title: 'Write data (instant, offline-safe)',
    code: `// Writes are local-first — no await, no fetch, no network wait
todos.set('todo:1:title', 'Buy milk');
todos.set('todo:1:done', false);
todos.set('todo:1:color', '#3b82f6');

// Reads are zero-latency — data lives in memory
const title = todos.get('todo:1:title'); // 'Buy milk'`,
  },
  {
    title: 'React hooks (auto-sync, auto-render)',
    code: `import { TopGunProvider, useMap } from '@topgunbuild/react';

function TodoList() {
  // Each field is its own key — gives per-field HLC timestamps
  const map = useMap<string, any>('sync-lab-todos');
  const title = map?.get('todo:1:title');
  const done = map?.get('todo:1:done');

  // Auto-re-renders on local writes AND remote merges
  return <div>{done ? '✓' : '○'} {title}</div>;
}`,
  },
];

/**
 * "How it's built" section showing minimal code snippets
 * demonstrating the TopGun API patterns used in this demo.
 */
export function CodeSnippets() {
  return (
    <section className="mt-8 border-t border-surface-light pt-8">
      <h2 className="mb-1 text-xl font-bold text-text">How it's built</h2>
      <p className="mb-6 text-text-muted">
        No Redux. No <code className="text-primary">await fetch</code>. No WebSocket listeners.
        Just <code className="text-primary">map.set()</code> and{' '}
        <code className="text-primary">useMap()</code>.
      </p>

      <div className="grid gap-6 md:grid-cols-3">
        {snippets.map(snippet => (
          <div key={snippet.title} className="rounded-lg bg-surface p-4">
            <h3 className="mb-3 text-sm font-semibold text-text">{snippet.title}</h3>
            <Highlight theme={themes.nightOwl} code={snippet.code} language="tsx">
              {({ style, tokens, getLineProps, getTokenProps }) => (
                <pre
                  className="overflow-x-auto rounded p-3 text-xs leading-relaxed"
                  style={style}
                >
                  {tokens.map((line, i) => (
                    <div key={i} {...getLineProps({ line })}>
                      {line.map((token, key) => (
                        <span key={key} {...getTokenProps({ token })} />
                      ))}
                    </div>
                  ))}
                </pre>
              )}
            </Highlight>
          </div>
        ))}
      </div>
    </section>
  );
}
