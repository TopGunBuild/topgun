import { EventEmitter } from 'events';
import { logger } from '../utils/logger';

export interface LockRequest {
  clientId: string;
  requestId: string;
  ttl: number;
  timestamp: number;
}

export interface LockState {
  name: string;
  owner: string; // clientId
  fencingToken: number;
  expiry: number;
  queue: LockRequest[];
}

export class LockManager extends EventEmitter {
  private locks: Map<string, LockState> = new Map();
  private checkInterval: NodeJS.Timeout;

  private static readonly MIN_TTL = 1000;   // 1 second
  private static readonly MAX_TTL = 300000; // 5 minutes

  constructor() {
    super();
    this.checkInterval = setInterval(() => this.cleanupExpiredLocks(), 1000);
  }

  public stop() {
    clearInterval(this.checkInterval);
  }

  public acquire(name: string, clientId: string, requestId: string, ttl: number): { granted: boolean; fencingToken?: number; error?: string } {
    // Validate TTL
    const safeTtl = Math.max(LockManager.MIN_TTL, Math.min(ttl || LockManager.MIN_TTL, LockManager.MAX_TTL));

    let lock = this.locks.get(name);
    if (!lock) {
      lock = {
        name,
        owner: '',
        fencingToken: 0,
        expiry: 0,
        queue: []
      };
      this.locks.set(name, lock);
    }

    const now = Date.now();

    // If lock is free or expired
    if (!lock.owner || lock.expiry < now) {
      this.grantLock(lock, clientId, safeTtl);
      return { granted: true, fencingToken: lock.fencingToken };
    }

    // If already owned by same client, extend lease
    if (lock.owner === clientId) {
      lock.expiry = Math.max(lock.expiry, now + safeTtl);
      logger.info({ name, clientId, fencingToken: lock.fencingToken }, 'Lock lease extended');
      return { granted: true, fencingToken: lock.fencingToken };
    }

    // Queue request
    lock.queue.push({ clientId, requestId, ttl: safeTtl, timestamp: now });
    logger.info({ name, clientId, queueLength: lock.queue.length }, 'Lock queued');
    return { granted: false };
  }

  public release(name: string, clientId: string, fencingToken: number): boolean {
    const lock = this.locks.get(name);
    if (!lock) return false;

    if (lock.owner !== clientId) {
      logger.warn({ name, clientId, owner: lock.owner }, 'Release failed: Not owner');
      return false;
    }

    if (lock.fencingToken !== fencingToken) {
      logger.warn({ name, clientId, sentToken: fencingToken, actualToken: lock.fencingToken }, 'Release failed: Token mismatch');
      return false;
    }

    this.processNext(lock);
    return true;
  }

  public handleClientDisconnect(clientId: string) {
    for (const lock of this.locks.values()) {
      // 1. If client owns the lock, force release
      if (lock.owner === clientId) {
        logger.info({ name: lock.name, clientId }, 'Releasing lock due to disconnect');
        this.processNext(lock);
      } else {
        // 2. Remove from queue if present
        const initialLen = lock.queue.length;
        lock.queue = lock.queue.filter(req => req.clientId !== clientId);
        if (lock.queue.length < initialLen) {
          logger.info({ name: lock.name, clientId }, 'Removed from lock queue due to disconnect');
        }
      }
    }
  }

  private grantLock(lock: LockState, clientId: string, ttl: number) {
    lock.owner = clientId;
    lock.expiry = Date.now() + ttl;
    lock.fencingToken++;
    logger.info({ name: lock.name, clientId, fencingToken: lock.fencingToken }, 'Lock granted');
  }

  private processNext(lock: LockState) {
    const now = Date.now();
    
    // Reset owner
    lock.owner = '';
    lock.expiry = 0;

    // Process queue
    while (lock.queue.length > 0) {
      const next = lock.queue.shift()!;
      
      // Grant to next
      this.grantLock(lock, next.clientId, next.ttl);
      
      // Emit event so ServerCoordinator can notify the client
      this.emit('lockGranted', {
        clientId: next.clientId,
        requestId: next.requestId,
        name: lock.name,
        fencingToken: lock.fencingToken
      });
      
      return;
    }
    
    // No one waiting
    if (lock.queue.length === 0) {
      this.locks.delete(lock.name);
    }
  }

  private cleanupExpiredLocks() {
    const now = Date.now();
    // Use a copy of keys to avoid concurrent modification issues during iteration
    const lockNames = Array.from(this.locks.keys());
    
    for (const name of lockNames) {
      const lock = this.locks.get(name);
      if (!lock) continue;

      if (lock.owner && lock.expiry < now) {
        logger.info({ name: lock.name, owner: lock.owner }, 'Lock expired, processing next');
        this.processNext(lock);
      } else if (!lock.owner && lock.queue.length === 0) {
        // Cleanup empty orphaned locks
        this.locks.delete(name);
      }
    }
  }
}

