import { DataStream } from '@topgunbuild/data-streams';
import { StoreResults, StoreValue, StoreWrapper } from '@topgunbuild/store';
import { SelectQuery, SelectOptions, Message, MessageHeader } from '@topgunbuild/transport';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { isNumber, debounce, randomBytes, toArray } from '@topgunbuild/utils';
import { ClientService } from '../client-service';
import { DataType } from '../types';
import { createStore } from '../utils/create-store';

const OUTPUT_TRIGGER_EVENT = 'output';
const INPUT_TRIGGER_EVENT  = 'input';

export class QueryHandler<D extends DataType, S extends SelectOptions> extends AsyncStreamEmitter<any>
{
    private queryStore: StoreWrapper;
    readonly service: ClientService;
    readonly query: SelectQuery;
    readonly dataStream: DataStream<D>;
    readonly options: S;
    readonly debounce: number;
    lastValue: D;
    once: boolean;

    get id(): string
    {
        return this.dataStream.name;
    }

    constructor(props: {
        service: ClientService,
        query: SelectQuery,
        options: S,
        debounce?: number
    })
    {
        super();
        this.service    = props.service;
        this.query      = props.query;
        this.options    = props.options;
        this.debounce   = props.debounce;
        this.dataStream = this.service.createDataStream<D>();
        this.service.setQueryHandler<D, S>(this);
        this._listenEvents(() => this.emit(OUTPUT_TRIGGER_EVENT, randomBytes()));
        this._initStore().then(() => this._fetchFirst());
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    preprocess(values: StoreValue[]|StoreValue): void
    {
        const filtered = toArray(values).filter(value => this.isQualify(value));
        if (filtered.length > 0)
        {
            this.process(filtered);
        }
    }

    process(values: StoreValue[]|StoreValue): void
    {
        this.emit(INPUT_TRIGGER_EVENT, toArray(values));
    }

    async destroy(): Promise<void>
    {
        this.dataStream.destroy();
        this.killAllListeners();
        await this.queryStore?.stop();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    protected isQualify(value: StoreValue): boolean
    {
        return false;
    }

    protected onOutput(results: StoreResults): void
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async _initStore(): Promise<void>
    {
        try
        {
            this.queryStore = await createStore(':memory:');
        }
        catch (e)
        {
        }
    }

    private async _fetchFirst(): Promise<void>
    {
        try
        {
            // Get local data
            if (this.options.local)
            {
                await this.service.waitForStoreInit();
                const result = await this.service.store.select(this.query);
                this.process(result.results);
            }

            // Request remote data
            if (this.options.remote)
            {
                const message = new Message({
                    header: new MessageHeader({}),
                    data  : this.query.encode(),
                });
                this.service.connectors.forEach(connector => connector.send(message));
            }
        }
        catch (e)
        {
        }
    }

    private _listenEvents(onChange: () => void): void
    {
        const debounceOnChange = isNumber(this.debounce)
            ? debounce(onChange, this.debounce)
            : onChange;

        (async () =>
        {
            for await (const values of this.listener(INPUT_TRIGGER_EVENT))
            {
                await this._inputHandler(values);
                debounceOnChange();
            }
        })();

        (async () =>
        {
            for await (const _ of this.listener(OUTPUT_TRIGGER_EVENT))
            {
                await this._outputHandler();
            }
        })();
    }

    private async _inputHandler(values: StoreValue[]): Promise<void>
    {
        try
        {
            await Promise.any(
                values.map(value => this.queryStore.put(value)),
            );
        }
        catch (e)
        {
        }
    }

    private async _outputHandler(): Promise<void>
    {
        try
        {
            const results = await this.queryStore.select(this.query);
            this.onOutput(results);
        }
        catch (e)
        {
        }
    }
}
