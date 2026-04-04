import { BaseAuthProvider } from './BaseAuthProvider';
import type { BaseAuthProviderConfig } from './BaseAuthProvider';

/**
 * Generic auth provider wrapping a user-supplied getToken function.
 * Escape hatch for Auth0, Supabase, or any other provider without a built-in helper.
 */
export class CustomAuthProvider extends BaseAuthProvider {
  private readonly getTokenFn: () => Promise<string | null>;

  constructor(getToken: () => Promise<string | null>, config?: BaseAuthProviderConfig) {
    super(config);
    this.getTokenFn = getToken;
  }

  protected async fetchExternalToken(): Promise<string | null> {
    return this.getTokenFn();
  }
}
