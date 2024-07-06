import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { DataNode, DataValue, StoreWrapper, toStoreValue } from '@topgunbuild/store';
import { isEmptyObject, isObject, mergeObjects } from '@topgunbuild/utils';
import {
    DeleteQuery,
    Message,
    MessageHeader,
    PutQuery, SelectOptions,
} from '@topgunbuild/transport';
import { bigintTime } from '@topgunbuild/time';
import { DataStream, Exchange } from '@topgunbuild/data-streams';
import { createConnector, Connector } from './transports';
import { PeerOptions, ClientOptions, DataType, ClientEvents } from './types';
import { createStore, getSocketOptions } from './utils';
import { QueryHandler } from './query-handlers';

export class ClientService
{
    readonly options: ClientOptions;
    readonly eventBus: AsyncStreamEmitter<any>;
    readonly connectors: Connector[];
    readonly exchange: Exchange;
    readonly queryHandlers: Map<string, QueryHandler<DataType, SelectOptions>>;
    store: StoreWrapper;

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

    createDataStream<T>(): DataStream<T>
    {
        return this.exchange.subscribe<T>();
    }

    setQueryHandler<T extends DataType, O extends SelectOptions>(handler: QueryHandler<T, O>): void
    {
        this.queryHandlers.set(handler.id, handler);
    }

    async putNode(section: string, node: string, value: DataNode): Promise<void>
    {
        if (!isObject(value))
        {
            throw new Error('Node must be an object.');
        }
        else if (isEmptyObject(value))
        {
            throw new Error('Node must not be an empty object.');
        }

        await Promise.all(
            Object.keys(value).map(field => this.putValue(section, node, field, value[field])),
        );
    }

    putValue(section: string, node: string, field: string, value: DataValue): Promise<void>
    {
        return this.#processDataChangeQuery(
            new PutQuery({
                section,
                node,
                field,
                value,
                state: bigintTime(),
            }),
        );
    }

    async delete(section: string, node: string, field?: string): Promise<void>
    {
        return this.#processDataChangeQuery(
            new DeleteQuery({
                section,
                node,
                field,
                state: bigintTime(),
            }),
        );
    }

    destroy(): void
    {
        this.exchange.destroy();
        this.queryHandlers.forEach(service => service.destroy());
        this.queryHandlers.clear();
    }

    async waitForStoreInit(): Promise<void>
    {
        if (!this.store)
        {
            await this.eventBus.listener(ClientEvents.storeInit).once();
        }
    }

    async #processDataChangeQuery(query: PutQuery|DeleteQuery): Promise<void>
    {
        const storeValue = toStoreValue(query);

        // Save to local store
        await this.waitForStoreInit();
        this.store.put(storeValue);

        // Save to query handlers
        this.queryHandlers.forEach(handler =>
        {
            handler.preprocess(storeValue);
        });

        const message = new Message({
            header: new MessageHeader({
            }),
            data  : query.encode(),
        });

        // Send to peers
        this.connectors.forEach(connector =>
        {
            connector.send(message, {
                once: true
            });
        });
    }

    async #initStore(): Promise<void>
    {
        this.store = await createStore(this.options.dbDirectory);
        this.eventBus.emit(ClientEvents.storeInit, this.store);
    }

    #initPeers(peers: PeerOptions[]): void
    {
        peers.forEach((peer: PeerOptions) =>
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
                if (this.queryHandlers.has(streamName))
                {
                    this.queryHandlers.delete(streamName);
                    await this.queryHandlers.get(streamName).destroy();
                }
            }
        })();
    }
}
