/**
 * TopGun real-time layer setup.
 *
 * Connects to the TopGun Rust server (running separately) and provides
 * helper functions for publishing task updates and managing presence.
 * The Express server calls these after REST mutations so that all
 * connected clients see changes in real time without polling.
 */

import { TopGunClient } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import type { LWWRecord } from '@topgunbuild/core';

// Minimal in-memory storage adapter for server-side use.
// The Express server does not need offline persistence -- Postgres is
// the source of truth. This adapter satisfies the TopGunClient interface.
class MemoryStorageAdapter implements IStorageAdapter {
  private store = new Map<string, any>();
  private meta = new Map<string, any>();

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  async get<V>(key: string): Promise<LWWRecord<V> | undefined> {
    return this.store.get(key);
  }
  async put(key: string, value: any): Promise<void> {
    this.store.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.store.delete(key);
  }
  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: any): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, any>): Promise<void> {
    for (const [k, v] of entries) this.store.set(k, v);
  }
  async appendOpLog(_entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    return 0;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return [];
  }
  async markOpsSynced(): Promise<void> {}
  async getAllKeys(): Promise<string[]> {
    return [...this.store.keys()];
  }
}

const TOPGUN_SERVER_URL = process.env.TOPGUN_SERVER_URL || 'ws://localhost:8080';

let client: TopGunClient | null = null;

/**
 * Initialise the TopGun client and connect to the Rust server.
 * Safe to call multiple times -- returns the existing client after first init.
 */
export function initTopGun(): TopGunClient {
  if (client) return client;

  client = new TopGunClient({
    serverUrl: TOPGUN_SERVER_URL,
    storage: new MemoryStorageAdapter(),
  });

  console.log(`[topgun] Connected to ${TOPGUN_SERVER_URL}`);
  return client;
}

/**
 * Publish a task update so all connected browser clients see the change
 * immediately via their WebSocket subscription.
 */
export function publishTaskUpdate(task: {
  id: number;
  title: string;
  status: string;
  assignee: string | null;
}): void {
  if (!client) return;
  client.topic('task-updates').publish({
    action: 'update',
    task,
    timestamp: Date.now(),
  });
}

/**
 * Publish a task deletion event.
 */
export function publishTaskDelete(taskId: number): void {
  if (!client) return;
  client.topic('task-updates').publish({
    action: 'delete',
    taskId,
    timestamp: Date.now(),
  });
}

/**
 * Update server-side presence for a user viewing a task.
 * Stores presence in a TopGun LWW map so all clients can read it.
 */
export function setPresence(userId: string, taskId: string | null): void {
  if (!client) return;
  const presence = client.getMap<string, any>('task-presence');
  if (taskId) {
    presence.set(userId, { taskId, since: Date.now() });
  } else {
    presence.set(userId, null);
  }
}

/**
 * Gracefully close the TopGun client connection.
 */
export async function closeTopGun(): Promise<void> {
  if (client) {
    await client.close();
    client = null;
  }
}
