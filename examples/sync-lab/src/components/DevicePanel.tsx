import { useState, useCallback, useRef } from 'react';
import type { LWWMap, LWWRecord } from '@topgunbuild/core';
import { useDeviceClient } from '@/hooks/useDeviceClient';
import { useLatencyTracker } from '@/hooks/useLatencyTracker';
import { useStateLog } from '@/hooks/useStateLog';
import { ControlPanel } from '@/components/ControlPanel';
import { StateLog } from '@/components/StateLog';
import { ConflictHighlight } from '@/components/ConflictHighlight';
import {
  todoKey,
  getRandomColor,
  detectMergeConflicts,
  collectTodoIds,
  type MergeResult,
  type TodoItem,
  formatTimestamp,
} from '@/lib/conflict-detector';

interface DevicePanelProps {
  deviceId: string;
  label: string;
  showTimestamps: boolean;
}

/**
 * A single "device" panel with its own TopGunClient, todo list,
 * disconnect/reconnect controls, and merge conflict visualization.
 */
export function DevicePanel({ deviceId, label, showTimestamps }: DevicePanelProps) {
  const {
    client,
    map,
    isConnected,
    todos,
    disconnect,
    reconnect,
    deviceId: id,
  } = useDeviceClient(deviceId);

  const { lastReadLatency, pendingOps } = useLatencyTracker(map, client);
  const { entries: logEntries, loggedSet, clear: clearLog } = useStateLog(map, client);

  const [newTitle, setNewTitle] = useState('');
  const [mergeResults, setMergeResults] = useState<Map<string, MergeResult>>(new Map());
  const [showLog, setShowLog] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState('');

  const preReconnectStateRef = useRef<Map<string, LWWRecord<any>> | null>(null);

  const handleAdd = useCallback(() => {
    if (!map || !newTitle.trim()) return;
    const id = crypto.randomUUID().slice(0, 8);
    loggedSet(map, todoKey(id, 'title'), newTitle.trim());
    loggedSet(map, todoKey(id, 'done'), false);
    loggedSet(map, todoKey(id, 'color'), getRandomColor());
    loggedSet(map, todoKey(id, '_exists'), true);
    setNewTitle('');
  }, [map, newTitle, loggedSet]);

  const handleToggleDone = useCallback(
    (todo: TodoItem) => {
      if (!map) return;
      loggedSet(map, todoKey(todo.id, 'done'), !todo.done);
    },
    [map, loggedSet],
  );

  const handleDelete = useCallback(
    (todo: TodoItem) => {
      if (!map) return;
      loggedSet(map, todoKey(todo.id, '_exists'), false);
    },
    [map, loggedSet],
  );

  const handleColorChange = useCallback(
    (todo: TodoItem) => {
      if (!map) return;
      loggedSet(map, todoKey(todo.id, 'color'), getRandomColor());
    },
    [map, loggedSet],
  );

  const handleEditStart = useCallback((todo: TodoItem) => {
    setEditingId(todo.id);
    setEditTitle(todo.title);
  }, []);

  const handleEditSave = useCallback(() => {
    if (!map || !editingId) return;
    loggedSet(map, todoKey(editingId, 'title'), editTitle);
    setEditingId(null);
    setEditTitle('');
  }, [map, editingId, editTitle, loggedSet]);

  const handleDisconnect = useCallback(() => {
    disconnect();
    setMergeResults(new Map());
  }, [disconnect]);

  const handleReconnect = useCallback(() => {
    // reconnect() returns the new map directly — avoids stale closure capture
    const { preState, newMap } = reconnect();
    preReconnectStateRef.current = preState;

    // After a short delay (let SyncEngine do its thing), detect conflicts
    setTimeout(() => {
      if (!preReconnectStateRef.current) return;
      const results = new Map<string, MergeResult>();
      const ids = collectTodoIds(newMap);
      for (const todoId of ids) {
        const result = detectMergeConflicts(preReconnectStateRef.current, newMap, todoId);
        if (result.conflicts.length > 0) {
          results.set(todoId, result);
        }
      }
      setMergeResults(results);
      preReconnectStateRef.current = null;
    }, 500);
  }, [reconnect]);

  return (
    <div className="flex flex-col rounded-xl border border-border bg-slate-800/50 p-4">
      {/* Header with device label and controls */}
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-lg font-semibold text-text">{label}</h3>
        <div className="flex gap-2">
          {isConnected ? (
            <button
              onClick={handleDisconnect}
              className="rounded bg-danger/20 px-3 py-1 text-xs font-medium text-danger hover:bg-danger/30 transition-colors"
            >
              Disconnect
            </button>
          ) : (
            <button
              onClick={handleReconnect}
              className="rounded bg-success/20 px-3 py-1 text-xs font-medium text-success hover:bg-success/30 transition-colors"
            >
              Reconnect
            </button>
          )}
        </div>
      </div>

      {/* Control panel */}
      <ControlPanel
        readLatency={lastReadLatency}
        pendingOps={pendingOps}
        isConnected={isConnected}
        deviceLabel={id}
      />

      {/* Add todo form */}
      <div className="mt-3 flex gap-2">
        <input
          type="text"
          value={newTitle}
          onChange={e => setNewTitle(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && handleAdd()}
          placeholder="Add a to-do..."
          disabled={!map}
          className="flex-1 rounded-md bg-surface px-3 py-2 text-sm text-text placeholder:text-text-muted focus:outline-none focus:ring-2 focus:ring-primary"
        />
        <button
          onClick={handleAdd}
          disabled={!map || !newTitle.trim()}
          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-white hover:bg-primary-dark disabled:opacity-50 transition-colors"
        >
          Add
        </button>
      </div>

      {/* Todo list */}
      <div className="mt-3 flex-1 space-y-1.5 overflow-y-auto">
        {todos.length === 0 ? (
          <p className="py-8 text-center text-sm text-text-muted">
            No to-dos yet. Add one above!
          </p>
        ) : (
          todos.map(todo => (
            <div key={todo.id} className="group animate-fade-in">
              <div className="flex items-center gap-2 rounded-lg bg-surface px-3 py-2">
                {/* Color dot */}
                <button
                  onClick={() => handleColorChange(todo)}
                  className="h-3 w-3 flex-shrink-0 rounded-full transition-transform hover:scale-125"
                  style={{ backgroundColor: todo.color }}
                  title="Change color"
                />

                {/* Checkbox */}
                <input
                  type="checkbox"
                  checked={todo.done}
                  onChange={() => handleToggleDone(todo)}
                  className="h-4 w-4 rounded border-border accent-primary"
                />

                {/* Title (editable) */}
                {editingId === todo.id ? (
                  <input
                    type="text"
                    value={editTitle}
                    onChange={e => setEditTitle(e.target.value)}
                    onKeyDown={e => e.key === 'Enter' && handleEditSave()}
                    onBlur={handleEditSave}
                    autoFocus
                    className="flex-1 rounded bg-surface-light px-2 py-0.5 text-sm text-text focus:outline-none"
                  />
                ) : (
                  <span
                    onClick={() => handleEditStart(todo)}
                    className={`flex-1 cursor-pointer text-sm ${
                      todo.done ? 'text-text-muted line-through' : 'text-text'
                    }`}
                  >
                    {todo.title}
                  </span>
                )}

                {/* HLC timestamp (visible when showTimestamps is on) */}
                {showTimestamps && map && (
                  <span className="text-[10px] font-mono text-text-muted">
                    {(() => {
                      const rec = map.getRecord(todoKey(todo.id, 'title'));
                      return rec ? formatTimestamp(rec.timestamp) : '';
                    })()}
                  </span>
                )}

                {/* Delete */}
                <button
                  onClick={() => handleDelete(todo)}
                  className="text-text-muted opacity-0 group-hover:opacity-100 hover:text-danger transition-all"
                >
                  <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>

              {/* Conflict highlights after merge */}
              {mergeResults.has(todo.id) && (
                <ConflictHighlight
                  conflicts={mergeResults.get(todo.id)!.conflicts}
                  showTimestamps={showTimestamps}
                />
              )}
            </div>
          ))
        )}
      </div>

      {/* State/Network log */}
      <StateLog
        entries={logEntries}
        visible={showLog}
        onToggle={() => setShowLog(v => !v)}
        onClear={clearLog}
      />
    </div>
  );
}
