import { TopGunClient, IDBAdapter } from '@topgunbuild/client';

// We'll initialize this with a token later
export const client = new TopGunClient({
    serverUrl: 'ws://localhost:4000', // Default dev port
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
