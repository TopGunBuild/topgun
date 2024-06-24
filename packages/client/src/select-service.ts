import { StoreValue, StoreWrapper } from '@topgunbuild/store';
import { DataStream } from '@topgunbuild/data-streams/src';
import { SelectMessage, SelectOptions } from '@topgunbuild/transport';
import { SelectCb } from './types';
import { createStore } from './utils/create-store';

export class SelectService
{
    memoryStore: StoreWrapper;
    dataStream: DataStream<any>;
    selectMessage: SelectMessage;
    local: boolean;
    remote: boolean;
    sync: boolean;
    cb?: SelectCb;

    static async create(
        dataStream: DataStream<any>,
        selectMessage: SelectMessage,
        selectOptions: SelectOptions,
        cb?: SelectCb,
    ): Promise<SelectService>
    {
        const store = await createStore(':memory:');
        return new SelectService(store, dataStream, selectMessage, selectOptions, cb);
    }

    constructor(
        store: StoreWrapper,
        dataStream: DataStream<any>,
        selectMessage: SelectMessage,
        selectOptions: SelectOptions,
        cb?: SelectCb,
    )
    {
        this.memoryStore   = store;
        this.dataStream    = dataStream;
        this.selectMessage = selectMessage;
        this.local         = selectOptions.local;
        this.remote        = selectOptions.remote;
        this.sync          = selectOptions.sync;
        this.cb            = cb;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

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
        await Promise.all(
            values.map(value => this.memoryStore.index.put(value)),
        );
        this.#triggerChanges();
    }

    async destroy(): Promise<void>
    {
        this.dataStream.destroy();
        await this.memoryStore?.stop();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #isQualify(value: StoreValue): boolean
    {
        return false;
    }

    #triggerChanges(): void
    {

    }
}
