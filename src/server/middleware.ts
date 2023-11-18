import { isObject, isString } from '@topgunbuild/typed';
import { TGSocketServer, RequestObject } from '@topgunbuild/socket/server';
import { TGServerOptions } from './server-options';
import { TGGraphAdapter, TGGraphData, TGMessage, TGNode, TGOptionsGet } from '../types';
import { pseudoRandomText } from '../sea';
import { TGExtendedLoggerType } from '../logger';
import { getNodeSoul, isNode } from '../utils';

export class Middleware
{
    /**
     * Constructor
     */
    constructor(
        private readonly serverName: string,
        private readonly socketServer: TGSocketServer,
        private readonly options: TGServerOptions,
        private readonly adapter: TGGraphAdapter,
        private readonly logger: TGExtendedLoggerType
    )
    {
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    setupMiddleware(): void
    {
        this.socketServer.addMiddleware(
            this.socketServer.MIDDLEWARE_SUBSCRIBE,
            this.#outboundMiddlewareHandler.bind(this)
        );

        this.socketServer.addMiddleware(
            this.socketServer.MIDDLEWARE_PUBLISH_IN,
            this.#inboundMiddlewareHandler.bind(this)
        );
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Handles inbound socket requests
     * Handle `put` queries
     */
    #inboundMiddlewareHandler(req: RequestObject): void
    {
        if (req.channel === 'topgun/put')
        {
            const msg = req.data as TGMessage;

            // Only allow if the connecting node was not an originator
            if (this.#originatorCheck(msg))
            {
                this.#processPut(msg).then((data) =>
                {
                    req.socket.transmit('#publish', {
                        channel: `topgun/@${msg['#']}`,
                        data
                    });
                })
            }
        }
    }

    /**
     * Handles all traffic out to connected sockets, this is always publish out
     * Handle `get` queries
     */
    async #outboundMiddlewareHandler(req: RequestObject): Promise<void>
    {
        if (req.channel === 'topgun/put')
        {
            return;
        }

        const soul = req.channel.replace(/^topgun\/nodes\//, '');

        if (!soul || soul === req.channel)
        {
            return;
        }

        const opts  = req.data as TGOptionsGet|undefined;
        const msgId = Math.random()
            .toString(36)
            .slice(2);

        this.#readNodes(opts)
            .then(graphData => this.#getRefNodes(graphData))
            .then(graphData => ({
                channel: req.channel,
                data   : {
                    '#'          : msgId,
                    'put'        : graphData,
                    'originators': { [this.serverName]: 1 }
                }
            }))
            .catch((e) =>
            {
                this.logger.warn(e.stack || e);
                return {
                    channel: req.channel,
                    data   : {
                        '#'  : msgId,
                        '@'  : req['#'],
                        'err': 'Error fetching node'
                    }
                }
            })
            .then((msg: {channel: string, data: TGMessage}) =>
            {
                req.socket.transmit('#publish', msg);
            })
    }

    /**
     * Get reference nodes if any
     */
    async #getRefNodes(graphData: TGGraphData): Promise<TGGraphData>
    {
        try
        {
            for (const soul in graphData)
            {
                if (!soul)
                {
                    continue;
                }

                const refSoul = graphData[soul] && graphData[soul]['#'];

                if (isString(refSoul))
                {
                    const refGraph     = await this.#readNodes({ ['#']: refSoul });
                    graphData[refSoul] = refGraph[refSoul];
                }
            }

            return graphData;
        }
        catch (e)
        {
            return graphData;
        }
    }

    /**
     * Check the originator attribute on the data to see if the intended target has already handled this data,
     * this is to prevent loop backs
     */
    #originatorCheck(msg: TGMessage)
    {
        return !(msg && msg.originators && msg.originators[this.serverName]);
    }

    #readNodes(opts: TGOptionsGet): Promise<TGGraphData>
    {
        return this.adapter.get(opts);
    }

    async #processPut(msg: TGMessage): Promise<TGMessage>
    {
        const msgId = pseudoRandomText();

        try
        {
            if (msg.put)
            {
                await this.adapter.put(msg.put, msg.originators);
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
}
