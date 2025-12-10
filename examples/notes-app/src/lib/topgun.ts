/// <reference types="vite/client" />
import { TopGunClient, EncryptedStorageAdapter } from '@topgunbuild/client';
import { IDBAdapter } from '@topgunbuild/adapters';

const SERVER_URL = import.meta.env.VITE_TOPGUN_SERVER_URL || 'ws://localhost:8080';

/**
 * Derives an AES-256-GCM encryption key from userId using PBKDF2.
 * The key is deterministic for the same userId, ensuring data can be decrypted
 * across sessions.
 */
async function deriveEncryptionKey(userId: string): Promise<CryptoKey> {
  const encoder = new TextEncoder();

  // Use userId as both the password and part of the salt for deterministic key derivation
  // In production, you might want to use a server-provided salt or user password
  const keyMaterial = await crypto.subtle.importKey(
    'raw',
    encoder.encode(userId),
    'PBKDF2',
    false,
    ['deriveBits', 'deriveKey']
  );

  // Derive the actual encryption key
  // Salt includes app identifier to make keys unique per application
  const salt = encoder.encode(`topgun-notes-app-${userId}`);

  return crypto.subtle.deriveKey(
    {
      name: 'PBKDF2',
      salt,
      iterations: 100000,
      hash: 'SHA-256'
    },
    keyMaterial,
    { name: 'AES-GCM', length: 256 },
    false, // not extractable
    ['encrypt', 'decrypt']
  );
}

// Client instance - will be initialized when user logs in
let tgClientInstance: TopGunClient | null = null;
let currentUserId: string | null = null;

/**
 * Gets or creates an encrypted TopGun client for the given user.
 * Returns null if userId is not provided.
 */
export async function getEncryptedClient(userId: string): Promise<TopGunClient> {
  // If we already have a client for this user, return it
  if (tgClientInstance && currentUserId === userId) {
    return tgClientInstance;
  }

  // If switching users, we need to recreate the client
  if (tgClientInstance && currentUserId !== userId) {
    console.log('Switching user, recreating encrypted client...');
    tgClientInstance = null;
    currentUserId = null;
  }

  console.log('Initializing encrypted TopGun client for user:', userId);

  // Derive encryption key from userId
  const encryptionKey = await deriveEncryptionKey(userId);

  // Create encrypted storage adapter
  const baseAdapter = new IDBAdapter();
  const encryptedAdapter = new EncryptedStorageAdapter(baseAdapter, encryptionKey);

  // Create client with encrypted storage
  tgClientInstance = new TopGunClient({
    serverUrl: SERVER_URL,
    storage: encryptedAdapter
  });

  currentUserId = userId;

  console.log('Encrypted TopGun client initialized successfully');
  return tgClientInstance;
}

/**
 * Gets the current client instance (may be null if not initialized).
 */
export function getClient(): TopGunClient | null {
  return tgClientInstance;
}

/**
 * Clears the client instance (e.g., on logout).
 */
export function clearClient(): void {
  tgClientInstance = null;
  currentUserId = null;
}

// Helper to update token when user logs in via Clerk
export const setTopGunAuth = (token: string) => {
  if (tgClientInstance) {
    tgClientInstance.setAuthToken(token);
  }
};
