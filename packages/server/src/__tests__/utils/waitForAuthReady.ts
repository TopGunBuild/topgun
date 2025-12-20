import { SyncEngine } from '@topgunbuild/client';
import { SyncState } from '@topgunbuild/client';

/**
 * Helper to wait for SyncEngine to be ready for authentication.
 * SyncEngine initiates connection in constructor, so we need to wait
 * for AUTHENTICATING state before calling setAuthToken.
 */
export async function waitForAuthReady(client: SyncEngine, timeout = 5000): Promise<void> {
    return new Promise((resolve, reject) => {
        let unsubscribe: (() => void) | undefined;

        const timer = setTimeout(() => {
            if (unsubscribe) unsubscribe();
            reject(new Error('Timeout waiting for AUTHENTICATING state'));
        }, timeout);

        const checkState = () => {
            const state = client.getConnectionState();
            if (state === SyncState.AUTHENTICATING ||
                state === SyncState.SYNCING ||
                state === SyncState.CONNECTED) {
                clearTimeout(timer);
                if (unsubscribe) unsubscribe();
                resolve();
            }
        };

        // Subscribe first so we don't miss state changes
        unsubscribe = client.onConnectionStateChange(() => {
            checkState();
        });

        // Check immediately in case already in correct state
        checkState();
    });
}
