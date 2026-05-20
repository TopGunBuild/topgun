import type {
  IConnectionProvider,
  IConnection,
  ConnectionProviderEvent,
  ConnectionEventHandler,
} from '../types';

/**
 * NullConnectionProvider implements IConnectionProvider for local-only mode.
 *
 * Used when neither serverUrl nor cluster is provided — the application
 * wants local persistence with no network sync target. All methods are
 * no-ops or explicit throws to fail loud rather than silently hang
 * when code paths that require a real connection are reached.
 */
export class NullConnectionProvider implements IConnectionProvider {
  connect(): Promise<void> {
    return Promise.resolve();
  }

  getConnection(_key: string): IConnection {
    throw new Error('NullConnectionProvider: no sync target configured');
  }

  getAnyConnection(): IConnection {
    throw new Error('NullConnectionProvider: no sync target configured');
  }

  isConnected(): boolean {
    return false;
  }

  getConnectedNodes(): string[] {
    return [];
  }

  on(_event: ConnectionProviderEvent, _handler: ConnectionEventHandler): void {
    // No-op: local-only mode has no connection events to emit
  }

  off(_event: ConnectionProviderEvent, _handler: ConnectionEventHandler): void {
    // No-op
  }

  send(_data: ArrayBuffer | Uint8Array, _key?: string): void {
    // No-op: local-only mode does not send data over the network
  }

  forceReconnect(): void {
    // No-op: no connection to reconnect
  }

  close(): Promise<void> {
    return Promise.resolve();
  }
}
