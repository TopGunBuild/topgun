import type { LWWMap, LWWRecord, Timestamp } from '@topgunbuild/core';

export interface TodoItem {
  id: string;
  title: string;
  done: boolean;
  color: string;
  exists: boolean;
}

export interface TodoField {
  value: any;
  timestamp: Timestamp;
}

export interface TodoWithTimestamps {
  id: string;
  fields: {
    title: TodoField;
    done: TodoField;
    color: TodoField;
    _exists: TodoField;
  };
}

export type ConflictStatus = 'matched' | 'resolved';

export interface FieldConflict {
  field: string;
  status: ConflictStatus;
  winningTimestamp: Timestamp;
  localValue: any;
  remoteValue: any;
}

export interface MergeResult {
  todoId: string;
  conflicts: FieldConflict[];
  hasConflicts: boolean;
}

const TODO_FIELDS = ['title', 'done', 'color', '_exists'] as const;

const TODO_COLORS = [
  '#2563eb', // blue
  '#ef4444', // red
  '#22c55e', // green
  '#eab308', // yellow
];

export function getRandomColor(): string {
  return TODO_COLORS[Math.floor(Math.random() * TODO_COLORS.length)];
}

/** Extract todo ID and field name from a composite key like "todo:abc123:title" */
export function parseTodoKey(key: string): { id: string; field: string } | null {
  const match = key.match(/^todo:([^:]+):([^:]+)$/);
  if (!match) return null;
  return { id: match[1], field: match[2] };
}

/** Build a composite key for a todo field */
export function todoKey(id: string, field: string): string {
  return `todo:${id}:${field}`;
}

/** Collect all unique todo IDs present in a map */
export function collectTodoIds(map: LWWMap<string, any>): Set<string> {
  const ids = new Set<string>();
  for (const key of map.allKeys()) {
    const parsed = parseTodoKey(key);
    if (parsed) ids.add(parsed.id);
  }
  return ids;
}

/** Reconstruct a single todo from its composite keys, returns null for deleted items */
export function reconstructTodo(map: LWWMap<string, any>, id: string): TodoItem | null {
  const existsRecord = map.getRecord(todoKey(id, '_exists'));
  if (!existsRecord || existsRecord.value === false || existsRecord.value === null) {
    return null;
  }
  return {
    id,
    title: map.get(todoKey(id, 'title')) ?? '',
    done: map.get(todoKey(id, 'done')) ?? false,
    color: map.get(todoKey(id, 'color')) ?? '#3b82f6',
    exists: true,
  };
}

/** Reconstruct a todo with full per-field HLC timestamps */
export function reconstructTodoWithTimestamps(
  map: LWWMap<string, any>,
  id: string,
): TodoWithTimestamps | null {
  const fields: Record<string, TodoField> = {};
  for (const field of TODO_FIELDS) {
    const record = map.getRecord(todoKey(id, field));
    if (record) {
      fields[field] = { value: record.value, timestamp: record.timestamp };
    } else {
      return null;
    }
  }
  return {
    id,
    fields: fields as TodoWithTimestamps['fields'],
  };
}

/** Get all non-deleted todos from a map */
export function getAllTodos(map: LWWMap<string, any>): TodoItem[] {
  const ids = collectTodoIds(map);
  const todos: TodoItem[] = [];
  for (const id of ids) {
    const todo = reconstructTodo(map, id);
    if (todo) todos.push(todo);
  }
  return todos;
}

/** Compare two HLC timestamps. Positive means a is newer. */
export function compareTimestamps(a: Timestamp, b: Timestamp): number {
  if (a.millis !== b.millis) return a.millis - b.millis;
  if (a.counter !== b.counter) return a.counter - b.counter;
  return a.nodeId.localeCompare(b.nodeId);
}

/**
 * Detect field-level conflicts between a pre-merge snapshot and the
 * current map state. Used after reconnect to show green (matched) or
 * yellow (LWW-resolved) highlights per field.
 */
export function detectMergeConflicts(
  beforeState: Map<string, LWWRecord<any>>,
  afterMap: LWWMap<string, any>,
  todoId: string,
): MergeResult {
  const conflicts: FieldConflict[] = [];

  for (const field of TODO_FIELDS) {
    const key = todoKey(todoId, field);
    const before = beforeState.get(key);
    const afterRecord = afterMap.getRecord(key);

    if (!before || !afterRecord) continue;

    const valuesMatch = JSON.stringify(before.value) === JSON.stringify(afterRecord.value);
    const timestampsMatch = compareTimestamps(before.timestamp, afterRecord.timestamp) === 0;

    if (valuesMatch && timestampsMatch) {
      conflicts.push({
        field,
        status: 'matched',
        winningTimestamp: afterRecord.timestamp,
        localValue: before.value,
        remoteValue: afterRecord.value,
      });
    } else {
      conflicts.push({
        field,
        status: 'resolved',
        winningTimestamp: afterRecord.timestamp,
        localValue: before.value,
        remoteValue: afterRecord.value,
      });
    }
  }

  return {
    todoId,
    conflicts,
    hasConflicts: conflicts.some(c => c.status === 'resolved'),
  };
}

/** Format a timestamp for display: millis_counter_nodeId */
export function formatTimestamp(ts: Timestamp): string {
  return `${ts.millis}_${ts.counter}_${ts.nodeId}`;
}
