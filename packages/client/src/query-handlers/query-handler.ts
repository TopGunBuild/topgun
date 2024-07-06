import { DataStream } from '@topgunbuild/data-streams';
import { StoreResults, StoreValue, StoreWrapper } from '@topgunbuild/store';
import { SelectQuery, SelectOptions, Message, MessageHeader } from '@topgunbuild/transport';
import { debounce, toArray } from '@topgunbuild/utils';
import { ClientService } from '../client-service';
import { DataType } from '../types';
import { createStore } from '../utils';
import { Queue } from '../control-flow';

export abstract class QueryHandler<D extends DataType, S extends SelectOptions>
{
    lastValue: D;
    once: boolean;
    queryStore: StoreWrapper;
    readonly service: ClientService;
    readonly query: SelectQuery;
    readonly dataStream: DataStream<D>;
    readonly options: S;
    readonly debounce: number;
    readonly inputQueue: Queue<StoreValue[]>;

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
        this.service    = props.service;
        this.query      = props.query;
        this.options    = props.options;
        this.debounce   = props.debounce || 0;
        this.dataStream = this.service.createDataStream<D>();
        this.inputQueue = new Queue<StoreValue[]>();
        this.service.setQueryHandler<D, S>(this);

        const onProcessedOutput = debounce(this.#onProcessedOutput.bind(this), this.debounce);

        (async () =>
        {
            for await (const value of this.inputQueue.listener('completed'))
            {
                await this.#onProcessedInput(value);
                onProcessedOutput();
            }
        })();

        this.#fetchFirst();
    }

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
        this.inputQueue.enqueue(toArray(values)).process();
    }

    async destroy(): Promise<void>
    {
        this.dataStream.destroy();
        await this.queryStore?.stop();
    }

    isQualify(value: StoreValue): boolean
    {
        return false;
    }

    onOutput(results: StoreResults): void
    {
    }

    async #fetchFirst(): Promise<void>
    {
        try
        {
            this.queryStore = await createStore(':memory:');

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
                this.service.connectors.forEach(connector =>
                {
                    connector.send(message, {
                        once: this.once,
                    });
                });
            }
        }
        catch (e)
        {
            console.error(e);
        }
    }

    async #onProcessedInput(values: StoreValue[]): Promise<void>
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

    async #onProcessedOutput(): Promise<void>
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
