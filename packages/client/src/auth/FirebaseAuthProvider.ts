import { BaseAuthProvider } from './BaseAuthProvider';
import type { BaseAuthProviderConfig } from './BaseAuthProvider';

export interface FirebaseAuth {
  currentUser?: {
    getIdToken: (forceRefresh?: boolean) => Promise<string>;
  } | null;
  onIdTokenChanged?: (callback: (user: any) => void) => () => void;
}

/**
 * Auth provider for Firebase. Delegates token fetching to Firebase Auth's currentUser.getIdToken().
 * Subscribes to onIdTokenChanged to invalidate cached tokens on session changes.
 */
export class FirebaseAuthProvider extends BaseAuthProvider {
  private readonly firebaseAuth: FirebaseAuth;
  private unsubscribeTokenChanged: (() => void) | null = null;

  constructor(firebaseAuth: FirebaseAuth, config?: BaseAuthProviderConfig) {
    super(config);
    this.firebaseAuth = firebaseAuth;
  }

  protected async fetchExternalToken(): Promise<string | null> {
    if (!this.firebaseAuth.currentUser) {
      return null;
    }
    return this.firebaseAuth.currentUser.getIdToken();
  }

  initialize(): void {
    if (this.firebaseAuth.onIdTokenChanged) {
      this.unsubscribeTokenChanged = this.firebaseAuth.onIdTokenChanged((user) => {
        if (user) {
          this.emit({ type: 'token:refreshed' });
        } else {
          this.emit({ type: 'auth:signedOut' });
        }
      });
    }
  }

  destroy(): void {
    if (this.unsubscribeTokenChanged) {
      this.unsubscribeTokenChanged();
      this.unsubscribeTokenChanged = null;
    }
    super.destroy();
  }
}
