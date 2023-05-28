import { TGSocketServer, TGSocket, RequestObject } from 'topgun-socket/server';
import { TGServerOptions } from './server-options';
import { TGGraphAdapter, TGMessage, TGNode } from '../types';
import { pseudoRandomText } from '../sea';

export class Middleware
{
    /**
     * Constructor
     */
    constructor(
        private readonly server: TGSocketServer,
        private readonly options: TGServerOptions,
        private readonly adapter: TGGraphAdapter,
    )
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    setupMiddleware(): void
    {
        this.server.addMiddleware(
            this.server.MIDDLEWARE_SUBSCRIBE,
            this.subscribeMiddleware.bind(this)
        );

        this.server.addMiddleware(
            this.server.MIDDLEWARE_PUBLISH_IN,
            this.publishInMiddleware.bind(this)
        );
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private publishInMiddleware(req: RequestObject): void
    {
        const msg = req.data;

        if (req.channel !== 'topgun/put')
        {
            return;
        }

        this.processPut(msg).then((data) =>
        {
            req.socket.transmit('#publish', {
                channel: `topgun/@${msg['#']}`,
                data
            });
        })
    }

    private async subscribeMiddleware(req: RequestObject): Promise<void>
    {
        if (req.channel === 'topgun/put')
        {
            return;
        }

        const soul = req.channel.replace(/^topgun\/nodes\//, '');

        if (!soul || soul === req.channel || soul === 'changelog')
        {
            return
        }

        const msgId = Math.random()
            .toString(36)
            .slice(2);

        this.readNode(soul)
            .then(node => ({
                channel: req.channel,
                data   : {
                    '#'  : msgId,
                    'put': node
                        ? {
                            [soul]: node
                        }
                        : null
                }
            }))
            .catch((e) =>
            {
                // tslint:disable-next-line: no-console
                console.warn(e.stack || e);
                return {
                    channel: req.channel,
                    data   : {
                        '#'  : msgId,
                        '@'  : req['#'],
                        'err': 'Error fetching node'
                    }
                }
            })
            .then((msg) =>
            {
                req.socket.transmit('#publish', msg);
            })
    }

    private readNode(soul: string): Promise<TGNode|null>
    {
        return this.adapter.get(soul);
    }

    private async processPut(msg: TGMessage): Promise<TGMessage>
    {
        const msgId = pseudoRandomText();

        try
        {
            if (msg.put)
            {
                await this.adapter.put(msg.put);
            }

            return {
                '#'  : msgId,
                '@'  : msg['#'],
                'err': null,
                'ok' : true,
            };
        }
        catch (e)
        {
            return {
                '#'  : msgId,
                '@'  : msg['#'],
                'err': 'Error saving',
                'ok' : false,
            };
        }
    }

    private isAdmin(socket: TGSocket): boolean|undefined
    {
        return (
            socket.authToken && socket.authToken.pub === this.options.ownerPub
        );
    }
}
