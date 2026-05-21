# @topgunbuild/react

[![npm](https://img.shields.io/npm/v/@topgunbuild/react)](https://www.npmjs.com/package/@topgunbuild/react) [![License](https://img.shields.io/npm/l/@topgunbuild/react)](https://github.com/TopGunBuild/topgun/blob/main/LICENSE)

React hooks for [TopGun](https://topgun.build) — build real-time apps that work offline, with no fetch state, no spinners, no manual subscriptions.

## Install

```bash
npm install @topgunbuild/react @topgunbuild/client @topgunbuild/adapters
```

Peer-requires `react >=16.8`.

## Quickstart

```tsx
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { TopGunClient } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';
import { TopGunProvider, useQuery, useMutation } from '@topgunbuild/react';

const client = new TopGunClient({
  storage: new IDBAdapter('my-app'),
  // serverUrl: 'ws://localhost:8080',   // optional
});
client.start();

function TodoList() {
  const { data: todos, loading } = useQuery<Todo>('todos');
  const { create, update } = useMutation<Todo>('todos');

  if (loading) return <p>Loading…</p>;

  return (
    <ul>
      {todos.map((todo) => (
        <li key={todo._key} onClick={() => update(todo._key, { ...todo, done: !todo.done })}>
          {todo.text}
        </li>
      ))}
      <button onClick={() => create(`todo-${Date.now()}`, { text: 'New', done: false })}>
        Add
      </button>
    </ul>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TopGunProvider client={client}>
      <TodoList />
    </TopGunProvider>
  </StrictMode>,
);
```

## Hooks

### `useQuery<T>(mapName, filter?, options?)`

Live-query a map. Re-renders whenever results change.

```tsx
const { data, loading, error, changes, hasMore } = useQuery<Todo>('todos', {
  where: { done: false },
  sort: { priority: 'desc' },
  limit: 20,
});
```

`data` is `Array<T & { _key: string }>`. The `_key` is the map key, useful as a React `key` and for mutations.

### `useMutation<T>(mapName)`

Optimistic writes against a map.

```tsx
const { create, update, remove } = useMutation<Todo>('todos');

create('todo-1', { text: 'Buy milk', done: false });
update('todo-1', { text: 'Buy milk', done: true });
remove('todo-1');
```

### `useMap<K, V>(name)` / `useORMap<K, V>(name)`

Returns the underlying CRDT and re-renders on any change. Use when you want lower-level access (iteration, `.get()`, `.size`).

```tsx
const todos = useMap<string, Todo>('todos');
return <p>You have {todos.size} todos.</p>;
```

### `useTopic(name, callback?)`

Subscribe to a pub/sub topic. The optional `callback` is invoked on every message.

```tsx
useTopic('notifications', (msg) => toast(msg.text));
```

### `usePNCounter(name)`

Reactive distributed counter.

```tsx
const { value, increment, decrement, add } = usePNCounter('likes');
return <button onClick={increment}>👍 {value}</button>;
```

### `useClient()`

Access the underlying `TopGunClient` (for `.search()`, `.sql()`, locks, etc.).

```tsx
const client = useClient();
const lock = client.getLock('migration');
```

## Documentation

- Full docs: [topgun.build/docs](https://topgun.build/docs)
- Client API: [`@topgunbuild/client`](https://www.npmjs.com/package/@topgunbuild/client)
- Live demo: [demo.topgun.build](https://demo.topgun.build)
- GitHub: [TopGunBuild/topgun](https://github.com/TopGunBuild/topgun)

## License

Apache-2.0
