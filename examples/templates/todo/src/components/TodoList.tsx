import { useState } from 'react';
import { useORMap, useClient } from '@topgunbuild/react';
import { TodoItem } from './TodoItem';

/**
 * Renders the full todo list using an ORMap for the collection so concurrent
 * additions from multiple tabs never lose a write (OR-Set semantics). Each
 * todo's editable fields live in a per-todo LWWMap inside <TodoItem>.
 */
export function TodoList() {
  const client = useClient();
  // ORMap tracks which todo IDs exist. OR-Set semantics mean two tabs can both
  // click "Add" simultaneously and neither write is lost — each gets a unique tag.
  const orMap = useORMap<string, string>('todos');
  const [newTitle, setNewTitle] = useState('');

  const todoIds = orMap.allKeys();

  function handleAdd() {
    const title = newTitle.trim();
    if (!title) return;
    const id = crypto.randomUUID();
    // Register the todo ID in the ORMap collection
    orMap.add(id, id);
    // Write the initial title directly into the per-todo LWWMap. We access the
    // map via the client (bypassing hooks) because we are inside an event handler,
    // not at the top level of a component. This is safe: the LWWMap is cached by
    // the client and TodoItem will see the same instance via useMap.
    const todoMap = client.getMap<string, string | boolean>(`todo:${id}`);
    todoMap.set('title', title);
    todoMap.set('completed', false);
    setNewTitle('');
  }

  function handleRemove(todoId: string) {
    // OR-Set remove: marks all currently-observed tags for this key as tombstones
    const values = orMap.get(todoId);
    for (const val of values) {
      orMap.remove(todoId, val);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter') handleAdd();
  }

  return (
    <div>
      {/* New todo input */}
      <div className="flex gap-2 mb-4">
        <input
          type="text"
          placeholder="Add a new todo…"
          value={newTitle}
          onChange={(e) => setNewTitle(e.target.value)}
          onKeyDown={handleKeyDown}
          className="flex-1 rounded-lg border border-gray-200 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-300"
        />
        <button
          onClick={handleAdd}
          disabled={!newTitle.trim()}
          className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-40 transition-colors"
        >
          Add
        </button>
      </div>

      {/* Todo items */}
      {todoIds.length === 0 ? (
        <p className="text-sm text-gray-400 text-center py-8">
          No todos yet — add one above or open another tab to see real-time sync.
        </p>
      ) : (
        <ul className="divide-y divide-gray-100">
          {todoIds.map((id) => (
            <TodoItem key={id} todoId={id} onRemove={handleRemove} />
          ))}
        </ul>
      )}
    </div>
  );
}
