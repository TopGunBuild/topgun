import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { StoreWrapper } from '@topgunbuild/store';
import { ClientOptions } from './options';
import * as sqlite from '@topgunbuild/sqlite';
import { StoreClient } from './store-client';
import { Ed25519Keypair } from '@topgunbuild/crypto';
import { ClientEvents } from './constants';

export class Client
{
    public readonly options: ClientOptions;

    #store: StoreWrapper;
    #eventBus: AsyncStreamEmitter<any>;

    constructor(options: ClientOptions)
    {
        const defaultOptions: ClientOptions = {
            peers   : [],
            identity: Ed25519Keypair.create(),
        };

        this.options   = Object.assign(defaultOptions, options || {});
        this.#eventBus = new AsyncStreamEmitter();
        this.start();
    }

    async start(): Promise<void>
    {
        let store = this.options.store;

        if (!store)
        {
            store = new sqlite.SQLLiteStore(sqlite, {
                directory: this.options.directory,
            });
            await store.start();
        }

        this.#store = new StoreWrapper(store);
        this.#eventBus.emit(ClientEvents.storeInit, this.#store);
    }

    get(path: string): StoreClient
    {
        return new StoreClient(path, this.#store, this.#eventBus);
    }

    async waitForStoreInit(): Promise<void>
    {
        if (!this.#store)
        {
            this.#store = await this.#eventBus.listener(ClientEvents.storeInit).once()
        }
    }
}
