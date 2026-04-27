// This app runs in offline-first local-only mode by default.
// To connect to a TopGun server, set serverUrl to a running ws:// endpoint.
// Cloud hosting is on the roadmap (see https://topgun.build/docs/roadmap).

import React, { useState } from 'react';
import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';
import { useQuery } from '@topgunbuild/react';

const client = new TopGunClient({
  storage: new IDBAdapter('topgun-app'),
  // Uncomment the line below to connect to a TopGun server:
  // serverUrl: 'ws://localhost:8080',
});

const todos = client.getMap('todos');

interface TodoItem {
  text: string;
  done: boolean;
}

export default function App() {
  const [input, setInput] = useState('');

  const { data: todoEntries } = useQuery<TodoItem>(todos);

  const addTodo = () => {
    const text = input.trim();
    if (!text) return;
    const id = `todo-${Date.now()}`;
    todos.set(id, { text, done: false });
    setInput('');
  };

  const toggleTodo = (id: string, item: TodoItem) => {
    todos.set(id, { ...item, done: !item.done });
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') addTodo();
  };

  return (
    <div style={{ fontFamily: 'system-ui, sans-serif', maxWidth: 480, margin: '60px auto', padding: '0 16px' }}>
      <h1 style={{ fontSize: '1.5rem', fontWeight: 700, marginBottom: 4 }}>TopGun Todo</h1>
      <p style={{ fontSize: '0.85rem', color: '#888', marginBottom: 24 }}>
        Offline-first · writes resolve locally · syncs when a server is available
      </p>

      <div style={{ display: 'flex', gap: 8, marginBottom: 20 }}>
        <input
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Add a task…"
          style={{
            flex: 1,
            padding: '8px 12px',
            border: '1px solid #ddd',
            borderRadius: 6,
            fontSize: '1rem',
          }}
        />
        <button
          onClick={addTodo}
          style={{
            padding: '8px 16px',
            background: '#111',
            color: '#fff',
            border: 'none',
            borderRadius: 6,
            cursor: 'pointer',
            fontSize: '1rem',
          }}
        >
          Add
        </button>
      </div>

      {todoEntries && todoEntries.length > 0 ? (
        <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
          {todoEntries.map(({ key, value }) => (
            <li
              key={key}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '10px 0',
                borderBottom: '1px solid #f0f0f0',
                cursor: 'pointer',
              }}
              onClick={() => toggleTodo(key, value)}
            >
              <input
                type="checkbox"
                checked={value.done}
                readOnly
                style={{ width: 16, height: 16, cursor: 'pointer' }}
              />
              <span
                style={{
                  textDecoration: value.done ? 'line-through' : 'none',
                  color: value.done ? '#aaa' : '#111',
                }}
              >
                {value.text}
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <p style={{ color: '#aaa', fontSize: '0.9rem' }}>No tasks yet. Add one above.</p>
      )}
    </div>
  );
}
