import { useMergeRejections } from '@topgunbuild/react';

/**
 * Secondary detail panel for technically curious visitors. Surfaces rejected
 * writes from the server-side conflict resolver via useMergeRejections. Only
 * receives events when the registered ConflictResolverDef rejects a merge.
 */
export function ConflictLog() {
  const { rejections, clear } = useMergeRejections({ mapName: 'todos', maxHistory: 20 });

  if (rejections.length === 0) {
    return null;
  }

  return (
    <section className="mt-6 border border-gray-200 rounded-lg p-4 bg-white">
      <div className="flex items-center justify-between mb-3">
        <h2 className="text-sm font-semibold text-gray-700">Conflict Log</h2>
        <button
          onClick={clear}
          className="text-xs text-gray-400 hover:text-gray-600 transition-colors"
        >
          Clear
        </button>
      </div>
      <ul className="space-y-2">
        {rejections.map((r, i) => (
          <li key={i} className="text-xs bg-red-50 border border-red-100 rounded p-2">
            <span className="font-medium text-red-700">{r.mapName}/{r.key}</span>
            {r.reason && (
              <span className="ml-2 text-red-500">— {r.reason}</span>
            )}
          </li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-gray-400">
        Rejections surface when the server-side resolver returns{' '}
        <code className="font-mono">{'{ action: "reject" }'}</code>. Silent LWW merges do not appear here.
      </p>
    </section>
  );
}
