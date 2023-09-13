import { WebSocketAdapter } from '../web-socket-adapter';
import { TGPeerOptions } from '../types';

export class TGPeers extends Map<string, WebSocketAdapter>
{
    /**
     * Constructor
     */
    constructor(peers?: TGPeerOptions[])
    {
        super();
        this.#init(peers);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    getEntries(): [string, WebSocketAdapter][]
    {
        return Array.from(this);
    }

    getPeerNames(): string[]
    {
        return Array.from(this.keys());
    }

    getOtherPeers(peerName: string): TGPeers
    {
        const peers = new TGPeers();

        this.forEach((value, key) =>
        {
            if (key !== peerName)
            {
                peers.set(key, value);
            }
        });

        return peers;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #init(peers?: TGPeerOptions[]): void
    {
        if (Array.isArray(peers))
        {
            for (const peer of peers)
            {
                const adapter = WebSocketAdapter.createByPeerOptions(peer);
                if (adapter?.baseUrl)
                {
                    this.set(adapter.baseUrl, adapter)
                }
            }
        }
    }
}
