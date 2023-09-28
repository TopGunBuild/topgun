import { TGPeerOptions } from '../types';
import { TGPeer } from './peer';

export class TGPeers extends Map<string, TGPeer>
{
    /**
     * Constructor
     */
    constructor(peers: TGPeerOptions[], peerSecretKey: string)
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

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #init(peers: TGPeerOptions[], peerSecretKey: string): void
    {
        if (Array.isArray(peers))
        {
            for (const peer of peers)
            {
                const adapter = new TGPeer(peer, peerSecretKey);
                if (adapter.uri)
                {
                    this.set(adapter.uri, adapter);
                }
            }
        }
    }
}
