import { BaseAuthProvider } from './BaseAuthProvider';
import type { BaseAuthProviderConfig } from './BaseAuthProvider';

export interface ClerkClient {
  session?: {
    getToken: () => Promise<string | null>;
  };
}

/**
 * Auth provider for Clerk. Delegates token fetching to the Clerk SDK session.
 * If no active session exists (clerkClient.session is undefined), getToken() returns null.
 */
export class ClerkAuthProvider extends BaseAuthProvider {
  private readonly clerkClient: ClerkClient;

  constructor(clerkClient: ClerkClient, config?: BaseAuthProviderConfig) {
    super(config);
    this.clerkClient = clerkClient;
  }

  protected async fetchExternalToken(): Promise<string | null> {
    if (!this.clerkClient.session) {
      return null;
    }
    return this.clerkClient.session.getToken();
  }
}
