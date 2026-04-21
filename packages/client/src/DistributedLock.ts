import { SyncEngine } from './SyncEngine';
import { logger } from './utils/logger';

export interface ILock {
  lock(ttl?: number): Promise<boolean>;
  unlock(): Promise<void>;
  isLocked(): boolean;
}

export class DistributedLock implements ILock {
  private syncEngine: SyncEngine;
  private name: string;
  private fencingToken: number | null = null;
  private _isLocked: boolean = false;

  constructor(syncEngine: SyncEngine, name: string) {
    this.syncEngine = syncEngine;
    this.name = name;
  }

  /**
   * Acquire the distributed lock.
   *
   * The `ttl` parameter is the server-side lease duration in milliseconds.
   * The client's response budget for waiting on the server grant is derived
   * by LockManager as `max(ttl + 5000ms grace, 5000ms minimum)`, so the client
   * will not reject before the server's TTL window elapses.
   *
   * @param ttl - Lock lease duration in milliseconds (server-side)
   * @returns Promise that resolves true on grant, false on failure
   */
  public async lock(ttl: number = 10000): Promise<boolean> {
    const requestId = crypto.randomUUID();
    try {
        const result = await this.syncEngine.requestLock(this.name, requestId, ttl);
        this.fencingToken = result.fencingToken;
        this._isLocked = true;
        return true;
    } catch (e) {
        return false;
    }
  }

  public async unlock(): Promise<void> {
      if (!this._isLocked || this.fencingToken === null) return;

      const requestId = crypto.randomUUID();
      try {
          const acked = await this.syncEngine.releaseLock(this.name, requestId, this.fencingToken);
          if (!acked) {
            logger.debug({ name: this.name, requestId }, 'DistributedLock: release not acknowledged by server');
          }
      } catch (e) {
          logger.debug({ name: this.name, requestId, error: (e as Error).message }, 'DistributedLock: release threw');
      } finally {
          this._isLocked = false;
          this.fencingToken = null;
      }
  }

  public isLocked(): boolean {
      return this._isLocked;
  }
}
