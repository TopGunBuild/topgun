import { TGSocketClientOptions } from '@topgunbuild/socket/client';
import { TGWebSocketGraphConnector } from '../client/transports/web-socket-graph-connector';
import { socketOptionsFromPeer } from '../utils/socket-options-from-peer';
import { TGGraphData, TGMessage, TGMessageCb, TGOptionsGet, TGOriginators } from '../types';
import { encrypt, work } from '../sea';
import { TGExtendedLoggerType } from '../logger';

export class TGPeer extends TGWebSocketGraphConnector
{
    readonly uri: string;

    /**
     * Constructor
     */
    constructor(
        private readonly peer: string|TGSocketClientOptions,
        private readonly peerSecretKey: string,
        private readonly logger: TGExtendedLoggerType
    )
    {
        super(socketOptionsFromPeer(peer), 'TGPeer');

        this.uri = this.client.transport.uri();
        this.#connectListener(peerSecretKey);
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

    waitForAuth(): Promise<void>
    {
        if (this.isAuthenticated())
        {
            return Promise.resolve();
        }

        return this.client.listener('authenticate').once();
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

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #connectListener(secret: string): Promise<void>
    {
        for await (const event of this.client.listener('connect'))
        {
            this.logger.debug('Peer is connected');
            try
            {
                await Promise.all([
                    this.#doAuth(secret),
                    this.client.listener('authenticate').once(),
                ]);
                this.logger.debug('Peer is auth!');
            }
            catch (e)
            {
                console.error(e.message);
            }
        }
    }

    /*async #authenticate(secret: string): Promise<void>
    {
        await this.waitForConnection();
        await this.#doAuth(secret);

        (async () =>
        {
            for await (const _event of this.client.listener('connect'))
            {
                this.#doAuth(secret);
            }
        })();
    }*/

    async #doAuth(secret: string): Promise<{channel: string; data: any}>
    {
        const id        = this.client.id;
        const timestamp = new Date().getTime();
        const challenge = `${id}/${timestamp}`;

        const hash = await work(challenge, secret);
        const data = await encrypt(JSON.stringify({ peerUri: this.uri }), hash, { raw: true });

        return this.client.invoke('peerLogin', { challenge, data });
    }
}