import { StrictMode } from 'react';
import ReactDOM from 'react-dom/client';
import { TopGunProvider } from '@topgunbuild/react';
import App, { client } from './App';

// Start the client so IndexedDB initializes and queued writes can persist.
// Non-blocking — UI renders immediately, persistence drains in the background.
client.start();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TopGunProvider client={client}>
      <App />
    </TopGunProvider>
  </StrictMode>,
);
