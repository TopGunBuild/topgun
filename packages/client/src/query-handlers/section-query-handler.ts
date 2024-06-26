import { DataNode, StoreValue, StoreWrapper } from '@topgunbuild/store';
import { SelectMessage, SelectSectionOptions } from '@topgunbuild/transport';
import { QueryHandler } from './query-handler';
import { createStore } from '../utils/create-store';
import { ClientService } from '../client-service';

export class SectionQueryHandler extends QueryHandler<DataNode[]>
{
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
        this.#createStore();
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

    async #createStore(): Promise<void>
    {
        this.memoryStore = await createStore(':memory:');
    }
}
