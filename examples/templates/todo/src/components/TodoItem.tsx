import { useState } from 'react';
import { useMap } from '@topgunbuild/react';

interface TodoItemProps {
  todoId: string;
  onRemove: (todoId: string) => void;
}

/**
 * Renders a single todo item backed by a per-todo LWWMap so that concurrent
 * edits from two tabs converge via HLC — the highest timestamp wins (LWW).
 * The server-side conflict resolver in conflictResolver.ts surfaces the losing
 * write via MERGE_REJECTED, which ConflictLog picks up via useMergeRejections.
 */
export function TodoItem({ todoId, onRemove }: TodoItemProps) {
  // One LWWMap per todo; keyed on todo ID so each item's fields resolve
  // independently when two tabs edit different todos simultaneously.
  const map = useMap<string, unknown>(`todo:${todoId}`);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');

  const title = typeof map.get('title') === 'string' ? (map.get('title') as string) : '(no title)';
  const completed = map.get('completed') === true;

  function handleToggle() {
    map.set('completed', !completed);
  }

  function handleEditStart() {
    setDraft(title === '(no title)' ? '' : title);
    setEditing(true);
  }

  function handleEditSave() {
    if (draft.trim()) {
      map.set('title', draft.trim());
    }
    setEditing(false);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter') handleEditSave();
    if (e.key === 'Escape') setEditing(false);
  }

  return (
    <li className="flex items-center gap-3 py-2 px-3 rounded-lg hover:bg-gray-50 group">
      <input
        type="checkbox"
        checked={completed}
        onChange={handleToggle}
        className="h-4 w-4 rounded border-gray-300 text-blue-600 cursor-pointer"
      />
      {editing ? (
        <input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={handleEditSave}
          onKeyDown={handleKeyDown}
          className="flex-1 rounded border border-blue-300 px-2 py-0.5 text-sm focus:outline-none focus:ring-1 focus:ring-blue-400"
        />
      ) : (
        <span
          onClick={handleEditStart}
          className={`flex-1 text-sm cursor-pointer select-none ${completed ? 'line-through text-gray-400' : 'text-gray-800'}`}
        >
          {title}
        </span>
      )}
      <button
        onClick={() => onRemove(todoId)}
        className="opacity-0 group-hover:opacity-100 text-gray-300 hover:text-red-400 transition-opacity text-xs px-1"
        aria-label="Remove todo"
      >
        ✕
      </button>
    </li>
  );
}
