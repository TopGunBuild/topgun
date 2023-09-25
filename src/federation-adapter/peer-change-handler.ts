import { TGMessage } from '../types';
import { PeersWriter } from './peers-writer';
import { TGPeer } from './peer';
import { TGExtendedLoggerType } from '../logger';

export class PeerChangeHandler
{
    disconnector: () => void;

    /**
     * Constructor
     */
    constructor(
        private readonly appName: string,
        private readonly peer: TGPeer,
        private readonly writer: PeersWriter,
        private readonly logger: TGExtendedLoggerType
    )
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    connect(): void
    {
        this.disconnector = this.peer.onChange((message: TGMessage) => this.#handlePeerChange(message));
    }

    disconnect(): void
    {
        this.disconnector && this.disconnector();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #handlePeerChange(changes: TGMessage): void
    {
        try
        {
            if (changes.originators && changes.originators[this.appName])
            {
                return;
            }
            this.writer.put(changes.put, this.peer.uri, changes.originators);
        }
        catch (e)
        {
            this.logger.error('Error syncing from peer', this.peer.uri, e.stack);
        }
    }
}
