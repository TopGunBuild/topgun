import { StrictMode } from 'react';
import ReactDOM from 'react-dom/client';
import { TopGunProvider } from '@topgunbuild/react';
import App, { client } from './App';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TopGunProvider client={client}>
      <App />
    </TopGunProvider>
  </StrictMode>,
);
