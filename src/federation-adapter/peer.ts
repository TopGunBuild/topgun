import { TGSocketClientOptions } from '@topgunbuild/socket/client';
import { TGWebSocketGraphConnector } from '../client/transports/web-socket-graph-connector';
import { socketOptionsFromPeer } from '../utils/socket-options-from-peer';
import { TGGraphData, TGOptionsGet } from '../types';

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

    putData(graph: TGGraphData): Promise<TGGraphData|null>
    {
        return new Promise<TGGraphData|null>((resolve) =>
        {
            this.put({
                graph,
                cb: (res: TGGraphData|null) => resolve(res)
            });
        });
    }

    getData(options: TGOptionsGet): Promise<TGGraphData>
    {
        return new Promise<TGGraphData>((resolve) =>
        {
            this.get({
                options,
                cb: (res: TGGraphData) => resolve(res)
            });
        });
    }

    onChange(cb: (res: TGGraphData) => void): () => void
    {
        const channel = this.subscribeToChannel('topgun/changelog', cb);

        return () =>
        {
            channel.unsubscribe();
        };
    }
}