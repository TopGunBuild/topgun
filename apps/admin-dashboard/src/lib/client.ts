import { TopGunClient, IDBAdapter } from '@topgunbuild/client';

// Use environment variable for server URL
// WebSocket connects to main server port (8080), not admin API port (9091)
const WS_URL = import.meta.env.VITE_WS_URL || 'ws://localhost:8080';

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
