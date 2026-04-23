import React from 'react';
import ReactDOM from 'react-dom/client';
import { TopGunProvider } from '@topgunbuild/react';
import { createTopGunClient } from '../../_shared/providerFactory';
import { todosConflictResolver } from './lib/conflictResolver';
import App from './App';
import './styles.css';

// Each app gets its own IndexedDB database name so data does not bleed between
// the todo and chat apps when both run simultaneously against the same origin.
const client = createTopGunClient('topgun-template-todo');

// Register the conflict resolver once before the UI mounts. Calling register()
// here (outside of any React component) avoids React 18 strict-mode
// double-registration that would occur inside a useEffect.
client.getConflictResolvers().register('todos', todosConflictResolver).catch((err: unknown) => {
  console.warn('[TopGun] Conflict resolver registration deferred (not yet connected):', err);
});

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <TopGunProvider client={client}>
      <App />
    </TopGunProvider>
  </React.StrictMode>,
);
