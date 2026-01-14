import { TopGunClient, IDBAdapter } from '@topgunbuild/client';

// Use environment variable for server URL, default to API_URL port for WebSocket
const WS_URL = import.meta.env.VITE_WS_URL ||
  (import.meta.env.VITE_API_URL
    ? import.meta.env.VITE_API_URL.replace(/^http/, 'ws')
    : 'ws://localhost:9090');

export const client = new TopGunClient({
  serverUrl: WS_URL,
  storage: new IDBAdapter(),
});

export const setAuthToken = (token: string) => {
  client.setAuthToken(token);
};

// Restore session from localStorage on module load
const savedToken = localStorage.getItem('topgun_token');
if (savedToken) {
  setAuthToken(savedToken);
}
