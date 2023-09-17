import { isFunction } from '@topgunbuild/typed';
import { TGPeers } from '../peers';
import { WebSocketAdapter } from '../../web-socket-adapter';
import { BatchWriter } from '../write/batch-writer';
import { TGGraphAdapter } from '../../types';
import { TGChangeSetEntry, TGFederatedAdapterOptions } from '../types';
import { Writer } from '../write/writer';
import { SyncWithPeer } from './sync-with-peer';
import { createSoul } from '../../utils';
import { PEER_SYNC_SOUL } from '../constants';

export class ConnectToPeer
{
    private readonly peer: WebSocketAdapter;
    private readonly otherPeers: TGPeers;
    private readonly batch: BatchWriter;
    private readonly syncWithPeer: SyncWithPeer;
    private batchTimeout: any;
    private lastKey: string;
    private syncedKey: string;
    disconnector: () => void;

    /**
     * Constructor
     */
    constructor(
        private readonly peerName: string,
        private readonly from: string,
        private readonly peers: TGPeers,
        private readonly persistence: TGGraphAdapter,
        private readonly options: TGFederatedAdapterOptions,
        private readonly writer: Writer
    )
    {
        this.peer = this.peers.get(peerName);

        if (!isFunction(this.peer?.onChange))
        {
            throw new Error(`Unconnectable peer ${peerName}`);
        }

        this.otherPeers   = this.peers.getOtherPeers(peerName);
        this.batch        = new BatchWriter(this.otherPeers, persistence, options, writer);
        this.syncWithPeer = new SyncWithPeer(peerName, from, peers, persistence, options, writer);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async connect(): Promise<ConnectToPeer>
    {
        // Catch up in batches before establishing connection
        this.lastKey      = await this.syncWithPeer.sync();
        this.syncedKey    = this.lastKey;
        this.disconnector = this.peer.onChange(this.#handlePeerChange.bind(this, this.lastKey));

        if (this.options.batchInterval)
        {
            this.#writeBatch();
        }

        return this;
    }

    disconnect(): void
    {
        this.disconnector && this.disconnector();
        this.batchTimeout && clearTimeout(this.batchTimeout);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #writeBatch(): Promise<void>
    {
        if (this.syncedKey === this.lastKey)
        {
            if (this.options.batchInterval)
            {
                this.#handleBatchTimeout();
            }
            return
        }

        this.syncedKey = this.lastKey;

        await this.batch.writeBatch();
        const peerSyncSoul = createSoul(PEER_SYNC_SOUL, this.peerName);
        await this.writer.internalPut({
            [peerSyncSoul]: {
                _: {
                    '#': peerSyncSoul,
                    '>': {
                        lastKey: new Date().getTime()
                    }
                },
                lastKey: this.lastKey
            }
        });

        if (this.options.batchInterval)
        {
            this.#handleBatchTimeout();
        }
    }

    #handleBatchTimeout(): void
    {
        this.batchTimeout = setTimeout(this.#writeBatch.bind(this), this.options.batchInterval);
    }

    #handlePeerChange([key, changes]: TGChangeSetEntry): void
    {
        try
        {
            this.batch.queueDiff(changes);
            this.lastKey = key;
            if (!this.options.batchInterval)
            {
                this.#writeBatch()
            }
        }
        catch (e)
        {
            console.warn('Error syncing from peer', this.peerName, e.stack)
        }
    }
}
