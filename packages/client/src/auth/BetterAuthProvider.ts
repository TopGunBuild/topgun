import { BaseAuthProvider } from './BaseAuthProvider';
import type { BaseAuthProviderConfig } from './BaseAuthProvider';

export interface BetterAuthClient {
  getSession: () => Promise<{ token: string } | null>;
}

/**
 * Auth provider for BetterAuth. Delegates token fetching to the BetterAuth client's getSession().
 * BetterAuth manages session refresh internally.
 */
export class BetterAuthProvider extends BaseAuthProvider {
  private readonly betterAuthClient: BetterAuthClient;

  constructor(betterAuthClient: BetterAuthClient, config?: BaseAuthProviderConfig) {
    super(config);
    this.betterAuthClient = betterAuthClient;
  }

  protected async fetchExternalToken(): Promise<string | null> {
    const session = await this.betterAuthClient.getSession();
    return session?.token ?? null;
  }
}
