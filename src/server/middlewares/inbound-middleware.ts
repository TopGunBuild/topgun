import {
    TGActionAuthenticate,
    TGActionInvoke,
    TGActionPublishIn,
    TGActionSubscribe,
    TGServerSocket
} from 'topgun-socket/server';
import { TGGraphAdapter, TGMessage, TGNode } from '../../types';
import { MiddlewareInboundStrategy } from './strategy/middleware-inbound-strategy';
import { pseudoRandomText } from '../../sea';
import { generateMessageId } from '../../client/graph/graph-utils';
import { TGServerOptions } from '../server-options';

export class InboundMiddleware extends MiddlewareInboundStrategy
{
    /**
     * Constructor
     */
    constructor(
        protected readonly adapter: TGGraphAdapter,
        private readonly options: TGServerOptions
    )
    {
        super();
    }

    onPublishIn(action: TGActionPublishIn): void
    {
        const msg = action.data;

        if (action.channel !== 'topgun/put')
        {
            if (this.isAdmin(action.socket))
            {
                action.allow();
            }
            else
            {
                action.block(new Error('You aren\'t allowed to write to this channel'));
            }
            return
        }

        action.allow();

        if (action.channel !== 'topgun/put' || !msg || !msg.put)
        {
            return
        }

        this.processPut(msg).then(data =>
        {
            this.publish(action, {
                channel: `topgun/@${msg['#']}`,
                data
            });
        })
    }

    async onSubscribe(action: TGActionSubscribe): Promise<void>
    {
        if (action.channel === 'topgun/put')
        {
            if (!this.isAdmin(action.socket))
            {
                action.block(new Error(`You aren't allowed to subscribe to ${action.channel}`));
                return
            }
        }

        const soul = String(action.channel).replace(/^topgun\/nodes\//, '');

        if (!soul || soul === action.channel)
        {
            action.allow();
            return;
        }

        action.allow();

        if (soul === 'changelog')
        {
            return;
        }

        const msgId = generateMessageId();

        this.readNode(soul)
            .then(node => ({
                channel: action.channel,
                data   : {
                    '#': msgId,
                    put: node
                        ? {
                            [soul]: node
                        }
                        : null
                }
            }))
            .catch(e =>
            {
                console.warn(e.stack || e);
                return {
                    channel: action.channel,
                    data   : {
                        '#': msgId,
                        '@': action.data['#'],
                        err: 'Error fetching node'
                    }
                }
            })
            // .then(res =>
            // {
            //     console.log({soul, res});
            //     return res;
            // })
            .then((msg: {channel: string, data: TGMessage}) => this.publish(action, msg))
    }

    default(
        action:
            |TGActionPublishIn
            |TGActionInvoke
            |TGActionSubscribe
            |TGActionAuthenticate
    ): void|Promise<void>
    {
        action.allow();
    }

    async onAuthenticate?(action: TGActionAuthenticate): Promise<void>
    {
        action.allow();
    }

    readNode(soul: string): Promise<TGNode|null>
    {
        return this.adapter.get(soul);
    }

    isAdmin(socket: TGServerSocket): boolean|undefined
    {
        return (
            socket.authToken && socket.authToken.pub === this.options.ownerPub
        );
    }

    /**
     * Persist put data and publish any resulting diff
     */
    async processPut(msg: TGMessage): Promise<TGMessage>
    {
        const msgId = pseudoRandomText();

        try
        {
            if (msg.put)
            {
                await this.adapter.put(msg.put);
            }

            return {
                '#': msgId,
                '@': msg['#'],
                err: null,
                ok : true
            }
        }
        catch (e)
        {
            return {
                '#': msgId,
                '@': msg['#'],
                err: 'Error saving',
                ok : false
            }
        }
    }

    publish(
        action:
            |TGActionPublishIn
            |TGActionInvoke
            |TGActionSubscribe
            |TGActionAuthenticate,
        message: {channel: string, data: TGMessage}
    ): void
    {
        action.socket.transmit('#publish', message, {});
    }
}
