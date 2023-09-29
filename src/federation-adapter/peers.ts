import { TGPeerOptions } from '../types';
import { TGPeer } from './peer';
import { TGExtendedLoggerType } from '../logger';

export class TGPeers extends Map<string, TGPeer>
{
    /**
     * Constructor
     */
    constructor(
        private readonly peers: TGPeerOptions[],
        private readonly peerSecretKey: string,
        private readonly logger: TGExtendedLoggerType
    )
    {
        super();
        this.#init(peers, peerSecretKey);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    getPeers(): TGPeer[]
    {
        return Array.from(this.values());
    }

    getOtherPeers(uri: string): TGPeer[]
    {
        return this.getPeers().filter(peer => peer.uri !== uri);
    }

    waitForAuth(): Promise<void[]>
    {
        return Promise.all(
            this.getPeers().map(peer => peer.waitForAuth())
        );
    }

    disconnect(): Promise<void[]>
    {
        return Promise.all(
            this.getPeers().map(peer => peer.disconnect())
        );
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #init(peers: TGPeerOptions[], peerSecretKey: string): void
    {
        if (Array.isArray(peers))
        {
            for (const peer of peers)
            {
                const adapter = new TGPeer(peer, peerSecretKey, this.logger);
                if (adapter.uri)
                {
                    this.set(adapter.uri, adapter);
                }
            }
        }
    }
}
