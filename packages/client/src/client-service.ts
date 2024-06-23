import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { DataNode, DataValue, StoreWrapper } from '@topgunbuild/store';
import { isEmptyObject, isObject, isFunction } from '@topgunbuild/utils';
import { Message, MessageHeader, PutMessage, SelectMessage } from '@topgunbuild/transport';
import { bigintTime } from '@topgunbuild/time';
import { DataStream, Exchange } from '@topgunbuild/data-streams/src';
import { Connector } from './transports/connector';
import { PeerOption, ClientOptions, SelectCb } from './types';
import { createConnector } from './transports/web-socket-connector';
import { getSocketOptions } from './utils/get-socket-options';
import { ClientEvents } from './constants';
import { QueryService } from './query-service';
import { createStore } from './utils/create-store';

export class ClientService
{
    public readonly options: ClientOptions;
    public readonly eventBus: AsyncStreamEmitter<any>;
    public readonly connectors: Connector[];
    public readonly exchange: Exchange;
    public store: StoreWrapper;

    #queryServices: Map<string, QueryService>;

    constructor(options: ClientOptions)
    {
        const defaultOptions: ClientOptions = {
            peers   : [],
            rowLimit: 1000,
        };

        this.options        = Object.assign(defaultOptions, options || {});
        this.eventBus       = new AsyncStreamEmitter();
        this.exchange       = new Exchange();
        this.connectors     = [];
        this.#queryServices = new Map();
        this.#initPeers(this.options.peers);
        this.#initStore();
        this.#handleListeners();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    select(query: SelectMessage, local: boolean, remote: boolean, cb?: SelectCb): DataStream<any>
    {
        const stream = this.exchange.subscribe();
        this.#handleQuery(stream, query, local, remote, cb);
        return stream;
    }

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
        this.#queryServices.forEach(service => service.destroy());
        this.#queryServices.clear();
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
                this.#queryServices.get(streamName)?.destroy();
                this.#queryServices.delete(streamName);
            }
        })();
    }

    async #handleQuery(
        stream: DataStream<any>,
        query: SelectMessage,
        local: boolean,
        remote: boolean,
        cb?: SelectCb,
    ): Promise<void>
    {
        const service = await QueryService.create(stream, query, local, remote, cb);
        this.#queryServices.set(stream.name, service);

        if (local)
        {
            await this.#waitForStoreInit();
            const result = await this.store.select(query);
            await service.putValues(result.results);
        }

        if (remote)
        {
            const message = new Message({
                header: new MessageHeader({}),
                data  : query.encode(),
            });
            this.connectors.forEach(connector => connector.send(message));
        }
    }
}
