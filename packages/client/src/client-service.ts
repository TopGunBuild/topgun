import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { DataNode, DataValue, StoreWrapper } from '@topgunbuild/store';
import { isEmptyObject, isObject, mergeObjects } from '@topgunbuild/utils';
import {
    Message,
    MessageHeader,
    PutMessage, SelectFieldOptions,
    SelectMessage, SelectNodeOptions,
    SelectSectionOptions,
} from '@topgunbuild/transport';
import { bigintTime } from '@topgunbuild/time';
import { DataStream, Exchange } from '@topgunbuild/data-streams';
import { Connector } from './transports/connector';
import { PeerOption, ClientOptions, QueryCb } from './types';
import { createConnector } from './transports/web-socket-connector';
import { getSocketOptions } from './utils/get-socket-options';
import { ClientEvents } from './constants';
import { createStore } from './utils/create-store';
import { QueryHandler } from './query-handlers/query-handler';
import { SectionQueryHandler } from './query-handlers/section-query-handler';
import { NodeQueryHandler } from './query-handlers/node-query-handler';
import { FieldQueryHandler } from './query-handlers/field-query-handler';

export class ClientService
{
    public readonly options: ClientOptions;
    public readonly eventBus: AsyncStreamEmitter<any>;
    public readonly connectors: Connector[];
    public readonly exchange: Exchange;
    public readonly queryHandlers: Map<string, QueryHandler<DataNode[]|DataNode|DataValue>>;
    public store: StoreWrapper;

    constructor(options: ClientOptions)
    {
        this.options = mergeObjects<ClientOptions>({
            peers   : [],
            rowLimit: 1000,
        }, options);

        this.eventBus      = new AsyncStreamEmitter();
        this.exchange      = new Exchange();
        this.connectors    = [];
        this.queryHandlers = new Map();
        this.#initPeers(this.options.peers);
        this.#initStore();
        this.#handleListeners();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    createDataStream<T>(): DataStream<T>
    {
        return this.exchange.subscribe<T>();
    }

    initQueryHandler<T extends DataNode[]|DataNode|DataValue>(handler: QueryHandler<T>): void
    {
        this.queryHandlers.set(handler.id, handler);
    }

    createSectionQueryHandler(options: SelectSectionOptions): SectionQueryHandler
    {
        const stream  = this.exchange.subscribe<DataNode[]>();
        const handler = new SectionQueryHandler(stream, options);
        this.queryHandlers.set(stream.name, handler);
        return handler;
    }

    createNodeQueryHandler(options: SelectNodeOptions): NodeQueryHandler
    {
        const stream  = this.exchange.subscribe<DataNode>();
        const handler = new NodeQueryHandler(stream, options);
        this.queryHandlers.set(stream.name, handler);
        return handler;
    }

    createFieldQueryHandler(options: SelectFieldOptions): FieldQueryHandler
    {
        const stream  = this.exchange.subscribe<DataValue>();
        const handler = new FieldQueryHandler(stream, options);
        this.queryHandlers.set(stream.name, handler);
        return handler;
    }

    #initQueryHandler<T extends QueryHandler<any>>()

    async putNode(sectionId: string, nodeId: string, value: DataNode): Promise<void>
    {
        if (this.authRequired())
        {
            throw new Error('You cannot save data to user space if the user is not authorized.');
        }
        else if (!isObject(value))
        {
            throw new Error('Node must be an object.');
        }
        else if (isEmptyObject(value))
        {
            throw new Error('Node must not be an empty object.');
        }

        await this.#waitForStoreInit();
        await Promise.all(
            Object.keys(value).map(field => this.putValue(sectionId, nodeId, field, value[field])),
        );
    }

    async putValue(sectionId: string, nodeId: string, field: string, value: DataValue): Promise<any>
    {
        const data    = new PutMessage(
            sectionId,
            nodeId,
            field,
            bigintTime(),
            value,
        );
        const message = new Message({
            header: new MessageHeader({}),
            data  : data.encode(),
        });

        // Save to local store
        this.store.put(data);

        // Send to peers
        this.connectors.forEach(connector => connector.send(message));
    }

    authRequired(): boolean
    {
        return false;
    }

    destroy(): void
    {
        this.exchange.destroy();
        // this.#queryServices.forEach(service => service.destroy());
        // this.#queryServices.clear();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #waitForStoreInit(): Promise<void>
    {
        if (!this.store)
        {
            this.store = await this.eventBus.listener(ClientEvents.storeInit).once();
        }
    }

    async #initStore(): Promise<void>
    {
        this.store = await createStore(this.options.dbDirectory);
        this.eventBus.emit(ClientEvents.storeInit, this.store);
    }

    #initPeers(peers: PeerOption[]): void
    {
        peers.forEach((peer: PeerOption) =>
        {
            try
            {
                const socketOpts = getSocketOptions(peer);

                if (socketOpts)
                {
                    this.connectors.push(createConnector(socketOpts));
                }
            }
            catch (e)
            {
                console.error(e);
            }
        });
    }

    #handleListeners(): void
    {
        (async () =>
        {
            // Unsubscribe from requests when link is destroyed
            for await (const { streamName } of this.exchange.listener('destroy'))
            {
                // this.#queryServices.get(streamName)?.destroy();
                // this.#queryServices.delete(streamName);
            }
        })();
    }

    async #handleSelect(
        dataStream: DataStream<any>,
        selectMessage: SelectMessage,
        selectOptions: SelectOptions,
        cb?: QueryCb,
    ): Promise<void>
    {
        const queryService = await SelectService.create(dataStream, selectMessage, selectOptions, cb);
        this.#queryServices.set(dataStream.name, queryService);

        if (selectOptions.local)
        {
            await this.#waitForStoreInit();
            const result = await this.store.select(selectMessage);
            await queryService.putValues(result.results);
        }

        if (selectOptions.remote)
        {
            const message = new Message({
                header: new MessageHeader({}),
                data  : selectMessage.encode(),
            });
            this.connectors.forEach(connector => connector.send(message));
        }
    }
}
