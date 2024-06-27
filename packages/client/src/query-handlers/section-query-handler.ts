import { DataNode, StoreValue, StoreWrapper } from '@topgunbuild/store';
import { SelectQuery, SelectSectionOptions } from '@topgunbuild/transport';
import { QueryHandler } from './query-handler';
import { createStore } from '../utils/create-store';
import { ClientService } from '../client-service';

export class SectionQueryHandler extends QueryHandler<DataNode[], SelectSectionOptions>
{
    memoryStore: StoreWrapper;
    memoryStoreInitEvent: string;

    constructor(props: {
        service: ClientService,
        query: SelectQuery,
        options: SelectSectionOptions,
    })
    {
        super(props);
        this.memoryStoreInitEvent = `ms-${this.id}-init`;
        this.fetchFirst();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async destroy(): Promise<void>
    {
        await super.destroy();
        await this.memoryStore?.stop();
    }

    async maybePutValues(values: StoreValue[]): Promise<void>
    {
        const filtered = values.filter(value => this.#isQualify(value));
        if (filtered.length)
        {
            await this.putValues(filtered);
        }
    }

    async putValues(values: StoreValue[]): Promise<void>
    {
        await this.#waitForMemoryStoreInit();
        await Promise.all(
            values.map(value => this.memoryStore.index.put(value)),
        );
        this.triggerChanges();
    }

    triggerChanges(): void
    {

    }

    async fetchFirst(): Promise<void>
    {
        await this.#createMemoryStore();
        await super.fetchFirst();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #isQualify(value: StoreValue): boolean
    {
        return false;
    }

    async #createMemoryStore(): Promise<void>
    {
        this.memoryStore = await createStore(':memory:');
        this.service.eventBus.emit(this.memoryStoreInitEvent, this.memoryStore);
    }

    async #waitForMemoryStoreInit(): Promise<void>
    {
        if (!this.memoryStore)
        {
            await this.service.eventBus.listener(this.memoryStoreInitEvent).once();
        }
    }
}
