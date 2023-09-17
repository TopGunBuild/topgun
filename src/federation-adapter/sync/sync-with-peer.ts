import { TGGraphAdapter } from '../../types';
import { TGChangeSetEntry, TGFederatedAdapterOptions } from '../types';
import { Writer } from '../write/writer';
import { WebSocketAdapter } from '../../web-socket-adapter';
import { TGPeers } from '../peers';
import { ChangesetFeed } from './changeset-feed';
import { BatchWriter } from '../write/batch-writer';
import { createSoul } from '../../utils';
import { PEER_SYNC_SOUL } from '../constants';

export class SyncWithPeer
{
    private readonly peer: WebSocketAdapter;
    private readonly otherPeers: TGPeers;
    private feed: ChangesetFeed;
    private batch: BatchWriter;
    private lastSeenKey: string;
    private entry: TGChangeSetEntry|null;

    static async sync(
        peerName: string,
        from: string,
        peers: TGPeers,
        persistence: TGGraphAdapter,
        options: TGFederatedAdapterOptions,
        writer: Writer
    ): Promise<string>
    {
        return new SyncWithPeer(peerName, from, peers, persistence, options, writer).sync();
    }

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
        this.peer        = this.peers.get(peerName);
        this.otherPeers  = this.peers.getOtherPeers(peerName);
        this.feed        = new ChangesetFeed(this.peer, this.from);
        this.batch       = new BatchWriter(this.otherPeers, this.persistence, this.options, this.writer);
        this.lastSeenKey = this.from;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async sync(): Promise<string>
    {
        while ((this.entry = await this.feed.getNext()))
        {
            const [key, changes] = this.entry;

            if (key > this.lastSeenKey)
            {
                this.batch.queueDiff(changes);
                this.lastSeenKey = key
            }
        }

        if (this.lastSeenKey > this.from)
        {
            await this.#writeBatch();
            await this.#saveSyncPoint();
        }

        return this.lastSeenKey;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #writeBatch(): Promise<void>
    {
        try
        {
            console.log('writing batch', this.peerName, this.lastSeenKey);
            await this.batch.writeBatch();
            console.log('wrote batch', this.peerName, this.lastSeenKey);
        }
        catch (e)
        {
            console.error('Error syncing with peer', this.peerName, e.stack);
        }
    }

    async #saveSyncPoint(): Promise<void>
    {
        const peerSyncSoul = createSoul(PEER_SYNC_SOUL, this.peerName);
        await this.writer.internalPut({
            [peerSyncSoul]: {
                _: {
                    '#': peerSyncSoul,
                    '>': {
                        lastSeenKey: new Date().getTime()
                    }
                },
                lastSeenKey: this.lastSeenKey
            }
        });
    }
}
