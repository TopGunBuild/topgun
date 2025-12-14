import { SyncEngine } from '@topgunbuild/client';
import { SyncState } from '@topgunbuild/client';

/**
 * Helper to wait for SyncEngine to be ready for authentication.
 * SyncEngine initiates connection in constructor, so we need to wait
 * for AUTHENTICATING state before calling setAuthToken.
 */
export async function waitForAuthReady(client: SyncEngine, timeout = 5000): Promise<void> {
    return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
            unsubscribe();
            reject(new Error('Timeout waiting for AUTHENTICATING state'));
        }, timeout);

        const checkState = () => {
            const state = client.getConnectionState();
            if (state === SyncState.AUTHENTICATING ||
                state === SyncState.SYNCING ||
                state === SyncState.CONNECTED) {
                clearTimeout(timer);
                unsubscribe();
                resolve();
            }
        };

        // Check immediately in case already in correct state
        checkState();

        const unsubscribe = client.onConnectionStateChange(() => {
            checkState();
        });
    });
}
