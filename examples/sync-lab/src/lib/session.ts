const SESSION_KEY = 'sync-lab-session-id';

let cachedSessionId: string | null = null;

/**
 * Get or create a session ID for this browser tab.
 * If ?session= query param is present, use that (for cross-device sharing).
 * Otherwise, generate a new one and store in sessionStorage (per-tab isolation).
 * Result is cached in-memory after first call to avoid redundant URL parsing.
 */
export function getSessionId(): string {
  if (cachedSessionId) {
    return cachedSessionId;
  }

  // Check URL for shared session param
  const params = new URLSearchParams(window.location.search);
  const sharedSession = params.get('session');
  if (sharedSession) {
    sessionStorage.setItem(SESSION_KEY, sharedSession);
    cachedSessionId = sharedSession;
    return sharedSession;
  }

  // Check sessionStorage for existing session
  const existing = sessionStorage.getItem(SESSION_KEY);
  if (existing) {
    cachedSessionId = existing;
    return existing;
  }

  // Generate new session ID (first 8 chars of UUID)
  const id = crypto.randomUUID().slice(0, 8);
  sessionStorage.setItem(SESSION_KEY, id);
  cachedSessionId = id;
  return id;
}

/**
 * Prefix a map name with the session namespace.
 * Example: prefixMap('sync-lab-todos') => 'sl-a1b2c3d4:sync-lab-todos'
 */
export function prefixMap(mapName: string): string {
  return `sl-${getSessionId()}:${mapName}`;
}

/**
 * Build a shareable URL with the current session ID.
 */
export function getShareUrl(): string {
  const url = new URL(window.location.href);
  url.searchParams.set('session', getSessionId());
  return url.toString();
}
