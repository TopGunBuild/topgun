import { ClientConfig, StorageAdapter } from "../types";
import { IndexedDBStorage } from "../storage/indexeddb-storage";
import { MemoryStorage } from "../storage/memory-storage";

export class Database
{
    private teamStorage: StorageAdapter<any>;
    private userStorage: StorageAdapter<any>;
    private keysetStorage: StorageAdapter<any>;
    private actionStorage: StorageAdapter<any>;
    private lockboxStorage: StorageAdapter<any>;
    private roleStorage: StorageAdapter<any>;
    private memberStorage: StorageAdapter<any>;
    private serverStorage: StorageAdapter<any>;
    private deviceStorage: StorageAdapter<any>;
    private messageStorage: StorageAdapter<any>;
    private invitationStorage: StorageAdapter<any>;

    constructor(private config: ClientConfig) {

    }

    createStorage<T>(storeName: string): StorageAdapter<T> {
        const storageConfig = { dbName: `topgun-${this.config.appId}`, storeName };

        if (this.config.storage) {
            return new this.config.storage(storageConfig);
        }

        if (IndexedDBStorage.isSupported()) {
            return new IndexedDBStorage(storageConfig);
        }

        return new MemoryStorage(storageConfig);
    }
}
