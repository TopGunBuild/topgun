import React, { useEffect, useState } from 'react';
import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';
import { TopGunProvider, useQuery, useMutation } from '@topgunbuild/react';

// Generated with: jwt.sign({ sub: "user-1" }, "topgun-secret-dev")
const VALID_DEV_TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLTEiLCJpYXQiOjE3NjM5MjYzNDJ9.CPJXaYYh0Otk-6gqxAtqiUkTaaoY4TtJ9zFgcPKEqZY";

interface TodoItem {
  id: string;
  text: string;
  completed: boolean;
  createdAt: number;
}

// Initialize Client Singleton
const adapter = new IDBAdapter();
const tgClient = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  storage: adapter
});

// Set auth token immediately (in real app, after login)
tgClient.setAuthToken(VALID_DEV_TOKEN);

export default function App() {
  const [isReady, setIsReady] = useState(false);

  useEffect(() => {
    const init = async () => {
      await tgClient.start();
      setIsReady(true);
    };
    init();
  }, []);

  if (!isReady) return <div>Loading TopGun...</div>;

  return (
    <TopGunProvider client={tgClient}>
      <div style={{ padding: '2rem', fontFamily: 'sans-serif', maxWidth: '600px', margin: '0 auto' }}>
        <h1>TopGun Offline Todo</h1>
        <TodoManager />
      </div>
    </TopGunProvider>
  );
}

function TodoManager() {
  const [filter, setFilter] = useState<'all' | 'active' | 'completed'>('all');
  const [inputText, setInputText] = useState('');
  
  // Build query based on UI filter
  const queryDef = filter === 'all' ? {} : { where: { completed: filter === 'completed' } };
  
  // Use the new React SDK hooks
  const { data: items, loading, error } = useQuery<TodoItem>('todos', queryDef);
  const { create, update, remove } = useMutation<TodoItem>('todos');

  const handleAdd = () => {
    if (!inputText.trim()) return;

    const id = crypto.randomUUID();
    const newItem: TodoItem = {
      id,
      text: inputText,
      completed: false,
      createdAt: Date.now()
    };

    create(id, newItem);
    setInputText('');
  };

  const handleToggle = (item: TodoItem) => {
    update(item.id, { ...item, completed: !item.completed });
  };

  const handleDelete = (id: string) => {
    remove(id);
  };

  // Sort by createdAt desc (client-side sort for now, though QueryHandle can sort too)
  const sortedItems = [...items].sort((a, b) => b.createdAt - a.createdAt);

  return (
    <>
      <div style={{ marginBottom: '1rem', padding: '0.5rem', background: '#f0f0f0', borderRadius: '4px' }}>
        Status: <strong>{loading ? 'Syncing...' : 'Ready'}</strong>
        {error && <div style={{ color: 'red' }}>Error: {error.message}</div>}
      </div>

      <div style={{ marginBottom: '1rem', display: 'flex', gap: '1rem' }}>
        <label>
          <input 
            type="radio" 
            name="filter" 
            checked={filter === 'all'} 
            onChange={() => setFilter('all')} 
          /> All
        </label>
        <label>
          <input 
            type="radio" 
            name="filter" 
            checked={filter === 'active'} 
            onChange={() => setFilter('active')} 
          /> Active
        </label>
        <label>
          <input 
            type="radio" 
            name="filter" 
            checked={filter === 'completed'} 
            onChange={() => setFilter('completed')} 
          /> Completed
        </label>
      </div>

      <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '2rem' }}>
        <input
          style={{ flex: 1, padding: '0.5rem' }}
          value={inputText}
          onChange={(e: React.ChangeEvent<HTMLInputElement>) => setInputText(e.target.value)}
          onKeyDown={(e: React.KeyboardEvent<HTMLInputElement>) => e.key === 'Enter' && handleAdd()}
          placeholder="Add a new task..."
        />
        <button onClick={handleAdd} style={{ padding: '0.5rem 1rem' }}>Add</button>
      </div>

      <ul style={{ listStyle: 'none', padding: 0 }}>
        {sortedItems.map(item => (
          <li key={item.id} style={{
            display: 'flex',
            alignItems: 'center',
            gap: '1rem',
            padding: '0.5rem',
            borderBottom: '1px solid #eee'
          }}>
            <input
              type="checkbox"
              checked={item.completed}
              onChange={() => handleToggle(item)}
            />
            <span style={{
              flex: 1,
              textDecoration: item.completed ? 'line-through' : 'none',
              color: item.completed ? '#999' : '#000'
            }}>
              {item.text}
            </span>
            <button onClick={() => handleDelete(item.id)} style={{ color: 'red' }}>
              Delete
            </button>
          </li>
        ))}
        {sortedItems.length === 0 && !loading && (
          <div style={{ color: '#999', textAlign: 'center' }}>No tasks found.</div>
        )}
      </ul>
    </>
  );
}
