import { TGGraphAdapter, TGGraphData } from '../types';
import { TGFederatedAdapterOptions } from './types';
import { Writer } from './writer';
import { TGPeer } from './peer';
import { TGExtendedLoggerType } from '../logger';

export class ConnectToPeer
{
    disconnector: () => void;

    /**
     * Constructor
     */
    constructor(
        private readonly peer: TGPeer,
        private readonly persistence: TGGraphAdapter,
        private readonly options: TGFederatedAdapterOptions,
        private readonly writer: Writer,
        private readonly logger: TGExtendedLoggerType
    )
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    connect(): void
    {
        this.disconnector = this.peer.onChange(this.#handlePeerChange.bind(this));
    }

    disconnect(): void
    {
        this.disconnector && this.disconnector();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #handlePeerChange(changes: TGGraphData): void
    {
        try
        {
            this.writer.put(changes, this.peer.uri);
        }
        catch (e)
        {
            this.logger.error('Error syncing from peer', this.peer.uri, e.stack);
        }
    }
}
