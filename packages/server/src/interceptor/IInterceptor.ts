import { LWWRecord, ORMapRecord, Principal } from '@topgunbuild/core';
import type { IWebSocketConnection } from '../transport';

export interface ServerOp {
  mapName: string;
  key: string;
  opType: 'PUT' | 'REMOVE' | 'OR_ADD' | 'OR_REMOVE';
  record?: LWWRecord<any>;
  orRecord?: ORMapRecord<any>;
  orTag?: string;
  id?: string; // Op ID if available
}

export interface ConnectionContext {
  clientId: string;
  socket?: IWebSocketConnection;
  principal?: Principal;
  isAuthenticated: boolean;
}

export interface OpContext extends ConnectionContext {
  fromCluster: boolean;
  originalSenderId?: string;
}

export interface IInterceptor {
  /**
   * Name of the interceptor for logging and debugging.
   */
  name: string;

  /**
   * Called when a new client connects.
   * Throwing an error here will reject the connection.
   */
  onConnection?(context: ConnectionContext): Promise<void>;

  /**
   * Called when a client disconnects.
   */
  onDisconnect?(context: ConnectionContext): Promise<void>;

  /**
   * Called before an operation is applied to the local map/storage.
   * Return the (possibly modified) op.
   * Return null to silently drop the operation (no error sent to client, no persistence).
   * Throwing an error will reject the operation and notify the client.
   */
  onBeforeOp?(op: ServerOp, context: OpContext): Promise<ServerOp | null>;

  /**
   * Called after an operation has been successfully applied and stored.
   */
  onAfterOp?(op: ServerOp, context: OpContext): Promise<void>;
}
