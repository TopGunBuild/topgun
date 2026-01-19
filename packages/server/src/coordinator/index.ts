// Coordinator modules - extracted from ServerCoordinator
// These modules handle focused responsibilities with clear interfaces

export { AuthHandler } from './auth-handler';
export { ConnectionManager } from './connection-manager';
export { StorageManager } from './storage-manager';
export { OperationHandler } from './operation-handler';
export type {
    // Auth types
    IAuthHandler,
    AuthHandlerConfig,
    AuthResult,
    // Connection types
    ClientConnection,
    IConnectionManager,
    ConnectionManagerConfig,
    // Storage types
    IStorageManager,
    StorageManagerConfig,
    // Operation types
    IOperationHandler,
    OperationHandlerConfig,
} from './types';
