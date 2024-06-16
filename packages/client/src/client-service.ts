import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { DataNode, DataValue, StoreWrapper } from '@topgunbuild/store';
import { Connector } from './transports/connector';
import { Ed25519Keypair } from '@topgunbuild/crypto';
import * as sqlite from '@topgunbuild/sqlite';
import { PeerOption, ClientOptions } from './types';
import { createConnector } from './transports/web-socket-connector';
import { getSocketOptions } from './utils/get-socket-options';
import { ClientEvents } from './constants';
import { isEmptyObject, isObject } from '@topgunbuild/utils';
import { Message, MessageHeader, PutMessage } from '@topgunbuild/transport';
import { bigintTime } from '@topgunbuild/time';

export class ClientService
{
    public readonly options: ClientOptions;
    public readonly eventBus: AsyncStreamEmitter<any>;
    public readonly connectors: Connector[];
    public store: StoreWrapper;

    constructor(options: ClientOptions)
    {
        const defaultOptions: ClientOptions = {
            peers   : [],
            identity: Ed25519Keypair.create(),
        };

        this.options    = Object.assign(defaultOptions, options || {});
        this.eventBus   = new AsyncStreamEmitter();
        this.connectors = [];
        this.initPeers(this.options.peers);
        this.initStore();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async waitForStoreInit(): Promise<void>
    {
        if (!this.store)
        {
            this.store = await this.eventBus.listener(ClientEvents.storeInit).once();
        }
    }

    async putNode(sectionId: string, nodeId: string, value: DataNode): Promise<void>
    {
        if (this.authRequired())
        {
            throw new Error(
                'You cannot save data to user space if the user is not authorized.',
            );
        }
        else if (!isObject(value))
        {
            throw new Error('Node must be an object.');
        }
        else if (isEmptyObject(value))
        {
            throw new Error('Node must not be an empty object.');
        }
        else if (!this.store)
        {
            await this.waitForStoreInit();
        }

        await Promise.all(
            Object.keys(value).map(field => this.putValue(sectionId, nodeId, field, value[field])),
        );
    }

    async putValue(sectionId: string, nodeId: string, field: string, value: DataValue): Promise<any>
    {
        const data = new PutMessage(
            sectionId,
            nodeId,
            field,
            bigintTime(),
            value,
        );

        this.store.put(data);

        const message = new Message({
            header: new MessageHeader({}),
            data: data.encode(),
        });
    }

    authRequired(): boolean
    {
        return false;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async initStore(): Promise<void>
    {
        let store = this.options.store;

        if (!store)
        {
            store = new sqlite.SQLLiteStore(sqlite, {
                directory: this.options.directory,
            });
            await store.start();
        }

        this.store = new StoreWrapper(store);
        this.eventBus.emit(ClientEvents.storeInit, this.store);
    }

    private initPeers(peers: PeerOption[]): void
    {
        peers.forEach((peer: PeerOption) =>
        {
            try
            {
                const socketOpts = getSocketOptions(peer);

                if (socketOpts)
                {
                    this.connectors.push(
                        createConnector(socketOpts),
                    );
                }
            }
            catch (e)
            {
                console.error(e);
            }
        });
    }
}
