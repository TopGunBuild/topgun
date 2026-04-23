import React from 'react';
import ReactDOM from 'react-dom/client';
import { TopGunProvider } from '@topgunbuild/react';
import { createTopGunClient } from '../../_shared/providerFactory';
import App from './App';
import './styles.css';

// Each app gets its own IndexedDB database name so data does not bleed between
// the todo and chat apps when both run simultaneously against the same origin.
const client = createTopGunClient('topgun-template-chat');

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <TopGunProvider client={client}>
      <App />
    </TopGunProvider>
  </React.StrictMode>,
);
