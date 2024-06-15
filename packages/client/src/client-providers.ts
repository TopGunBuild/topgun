import { AsyncStreamEmitter } from '@topgunbuild/async-stream-emitter';
import { StoreWrapper } from '@topgunbuild/store';
import { Connector } from './transports/connector';
import { Ed25519Keypair } from '@topgunbuild/crypto';
import * as sqlite from '@topgunbuild/sqlite';
import { PeerOption, ClientOptions } from './types';
import { createConnector } from './transports/web-socket-connector';
import { getSocketOptions } from './utils/get-socket-options';
import { ClientEvents } from './constants';

export class ClientProviders
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
        this.peersInit(this.options.peers);
        this.storeInit();
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

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private async storeInit(): Promise<void>
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

    private peersInit(peers: PeerOption[]): void
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
