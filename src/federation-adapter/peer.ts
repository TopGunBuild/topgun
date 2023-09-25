import { TGSocketClientOptions } from '@topgunbuild/socket/client';
import { TGWebSocketGraphConnector } from '../client/transports/web-socket-graph-connector';
import { socketOptionsFromPeer } from '../utils/socket-options-from-peer';
import { TGGraphData, TGMessage, TGMessageCb, TGOptionsGet, TGOriginators } from '../types';

export class TGPeer extends TGWebSocketGraphConnector
{
    readonly uri: string;

    /**
     * Constructor
     */
    constructor(peer: string|TGSocketClientOptions, name: string = 'TGPeer')
    {
        super(socketOptionsFromPeer(peer), name);

        this.uri = this.client.transport.uri();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    isOpen(): boolean
    {
        return this.client.state === this.client.OPEN;
    }

    isAuthenticated(): boolean
    {
        return this.client.authState === this.client.AUTHENTICATED;
    }

    putInPeer(graph: TGGraphData, originators: TGOriginators): Promise<TGMessage>
    {
        return new Promise<TGMessage>((resolve) =>
        {
            this.put({
                graph,
                originators,
                cb: (res: TGMessage) => resolve(res)
            });
        });
    }

    getFromPeer(options: TGOptionsGet): Promise<TGMessage>
    {
        return new Promise<TGMessage>((resolve) =>
        {
            this.get({
                options,
                cb: (res: TGMessage) => resolve(res)
            });
        });
    }

    onChange(cb: TGMessageCb): () => void
    {
        const channel = this.subscribeToChannel('topgun/changelog', cb);

        return () =>
        {
            channel.unsubscribe();
        };
    }
}