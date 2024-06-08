import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { StoreWrapper } from '@topgunbuild/store';
import { ClientEvents } from './constants';

export class StoreClient
{
    readonly path: string;
    #store: StoreWrapper;
    #eventBus: AsyncStreamEmitter<any>;

    constructor(path: string, store: StoreWrapper, eventBus: AsyncStreamEmitter<any>)
    {
        this.path      = path;
        this.#store    = store;
        this.#eventBus = eventBus;
    }

    async put()
    {
        await this.waitForStoreInit();
    }

    async search()
    {
        await this.waitForStoreInit();
    }

    private async waitForStoreInit(): Promise<void>
    {
        if (!this.#store)
        {
            this.#store = await this.#eventBus.listener(ClientEvents.storeInit).once()
        }
    }
}
