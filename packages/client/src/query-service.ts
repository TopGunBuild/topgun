import { StoreValue, StoreWrapper } from '@topgunbuild/store';
import { DataStream } from '@topgunbuild/data-streams/src';
import { SelectMessage } from '@topgunbuild/transport';
import { SelectCb } from './types';
import { createStore } from './utils/create-store';

export class QueryService
{
    store: StoreWrapper;
    stream: DataStream<any>;
    query: SelectMessage;
    local: boolean;
    remote: boolean;
    cb?: SelectCb;

    static async create(
        stream: DataStream<any>,
        query: SelectMessage,
        local: boolean,
        remote: boolean,
        cb?: SelectCb,
    ): Promise<QueryService>
    {
        const store = await createStore(':memory:');
        return new QueryService(store, stream, query, local, remote, cb);
    }

    constructor(
        store: StoreWrapper,
        stream: DataStream<any>,
        query: SelectMessage,
        local: boolean,
        remote: boolean,
        cb?: SelectCb,
    )
    {
        this.store  = store;
        this.stream = stream;
        this.query  = query;
        this.local  = local;
        this.remote = remote;
        this.cb     = cb;
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
            values.map(value => this.store.index.put(value)),
        );
        this.#triggerChanges();
    }

    async destroy(): Promise<void>
    {
        this.stream.destroy();
        await this.store?.stop();
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
