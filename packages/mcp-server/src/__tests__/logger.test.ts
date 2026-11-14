import { configForLog } from '../logger';

describe('configForLog', () => {
  it('keeps only the named safe fields and no credential', () => {
    const config = {
      topgunUrl: 'wss://user:url-password@db.example:8443/path?token=url-query-secret',
      authToken: 'auth-token-secret',
      allowedMaps: ['tasks'],
      enableMutations: false,
      enableSubscriptions: true,
      debug: true,
      // A field the logger does not know about must not be logged.
      refreshToken: 'refresh-token-secret',
    };
    const logged = configForLog(config);
    const text = JSON.stringify(logged);

    for (const secret of [
      'url-password',
      'url-query-secret',
      'auth-token-secret',
      'refresh-token-secret',
    ]) {
      expect(text).not.toContain(secret);
    }
    expect(logged).toEqual({
      topgunOrigin: 'wss://db.example:8443',
      authTokenSet: true,
      allowedMaps: ['tasks'],
      enableMutations: false,
      enableSubscriptions: true,
      debug: true,
    });
  });

  it('reports an unset token and an unparseable URL without echoing it', () => {
    const logged = configForLog({ topgunUrl: 'not a url with secret-bits' });
    expect(logged.authTokenSet).toBe(false);
    expect(logged.topgunOrigin).toBe('[unparseable]');
  });
});
