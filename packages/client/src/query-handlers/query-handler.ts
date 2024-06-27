import { DataStream } from '@topgunbuild/data-streams';
import { StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectOptions, Message, MessageHeader } from '@topgunbuild/transport';
import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { ClientService } from '../client-service';
import { DataType, QueryHandlerEvents } from '../types';

export class QueryHandler<T extends DataType, O extends SelectOptions>
{
    service: ClientService;
    query: SelectQuery;
    dataStream: DataStream<T>;
    options: O;
    once: boolean;
    eventBus: AsyncStreamEmitter<any>;

    get id(): string
    {
        return this.dataStream.name;
    }

    constructor(props: {
        service: ClientService,
        query: SelectQuery,
        options: O
    })
    {
        this.service    = props.service;
        this.query      = props.query;
        this.dataStream = this.service.createDataStream<T>();
        this.options    = props.options;
        this.eventBus   = new AsyncStreamEmitter();
        this.service.initQueryHandler<T, O>(this);
    }

    async destroy(): Promise<void>
    {
        this.dataStream.destroy();
        this.eventBus.killAllListeners();
    }

    protected async maybePutValues(values: StoreValue[]): Promise<void>
    {
    }

    protected async putValues(values: StoreValue[], fromLocal: boolean): Promise<void>
    {
    }

    protected triggerChanges(): void
    {
    }

    protected async fetchFirst(): Promise<void>
    {
        // Get local data
        if (this.options.local)
        {
            await this.service.waitForStoreInit();
            const result = await this.service.store.select(this.query);
            // this.eventBus.emit(QueryHandlerEvents.localDataFetched, result.results);
            await this.putValues(result.results, true);
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
}
