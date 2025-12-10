import { SyncEngine } from './SyncEngine';

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
          await this.syncEngine.releaseLock(this.name, requestId, this.fencingToken);
      } finally {
          this._isLocked = false;
          this.fencingToken = null;
      }
  }

  public isLocked(): boolean {
      return this._isLocked;
  }
}

