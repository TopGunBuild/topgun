import { DataNode, StoreValue, StoreWrapper } from '@topgunbuild/store';
import { Message, MessageHeader, SelectMessage, SelectSectionOptions } from '@topgunbuild/transport';
import { QueryHandler } from './query-handler';
import { createStore } from '../utils/create-store';
import { ClientService } from '../client-service';

export class SectionQueryHandler extends QueryHandler<DataNode[]>
{
    service: ClientService;
    memoryStore: StoreWrapper;
    selectMessage: SelectMessage;

    constructor(props: {
        service: ClientService,
        options: SelectSectionOptions,
        message: SelectMessage
    })
    {
        super(props.service, props.options.local, props.options.remote, props.options.sync);
        this.selectMessage = props.message;
        this.service       = props.service;
        this.#fetchFirst();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async maybePutValues(values: StoreValue[]): Promise<void>
    {
        const filtered = values.filter(value => this.#isQualify(value));
        if (filtered.length)
        {
            await this.#putValues(filtered);
        }
    }

    async destroy(): Promise<void>
    {
        await super.destroy();
        await this.memoryStore?.stop();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #putValues(values: StoreValue[]): Promise<void>
    {
        await Promise.all(
            values.map(value => this.memoryStore.index.put(value)),
        );
        this.#triggerChanges();
    }

    #isQualify(value: StoreValue): boolean
    {
        return false;
    }

    #triggerChanges(): void
    {

    }

    async #fetchFirst(): Promise<void>
    {
        this.memoryStore = await createStore(':memory:');

        // Get local data
        if (this.local)
        {
            await this.service.waitForStoreInit();
            const result = await this.service.store.select(this.selectMessage);
            await this.#putValues(result.results);
            this.#triggerChanges();
        }

        // Request remote data
        if (this.remote)
        {
            const message = new Message({
                header: new MessageHeader({}),
                data  : this.selectMessage.encode(),
            });
            this.service.connectors.forEach(connector => connector.send(message));
        }
    }
}
