/**
 * Generates and persists a stable guest identity in localStorage so the same
 * browser tab always appears as the same user across page reloads without
 * requiring authentication.
 */

const STORAGE_KEY = 'topgun-guest-identity';

export interface GuestIdentity {
  guestId: string;
  displayName: string;
}

function generateId(): string {
  return crypto.randomUUID();
}

function generateDisplayName(): string {
  const adjectives = ['Swift', 'Quiet', 'Bold', 'Calm', 'Keen', 'Wise', 'Brave'];
  const nouns = ['Falcon', 'River', 'Stone', 'Cloud', 'Ember', 'Spark', 'Wave'];
  const adj = adjectives[Math.floor(Math.random() * adjectives.length)];
  const noun = nouns[Math.floor(Math.random() * nouns.length)];
  const num = Math.floor(Math.random() * 100);
  return `${adj}${noun}${num}`;
}

/**
 * Returns the persisted guest identity for this browser, creating one on first visit.
 */
export function getGuestIdentity(): GuestIdentity {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored) {
    try {
      const parsed = JSON.parse(stored) as GuestIdentity;
      if (parsed.guestId && parsed.displayName) {
        return parsed;
      }
    } catch {
      // Corrupted storage — fall through to create a new identity
    }
  }

  const identity: GuestIdentity = {
    guestId: generateId(),
    displayName: generateDisplayName(),
  };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(identity));
  return identity;
}
