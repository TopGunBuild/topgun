import {
    TGActionAuthenticate,
    TGActionInvoke,
    TGActionPublishIn,
    TGActionSubscribe,
    TGActionTransmit,
} from 'topgun-socket/server';

export abstract class MiddlewareInboundStrategy
{
    /**
     * When server receives a transmit action
     */
    onTransmit?(action: TGActionTransmit): void|Promise<void>;

    /**
     * Invoke type
     */
    onInvoke?(action: TGActionInvoke): void|Promise<void>;

    /**
     * Publish_in type
     */
    onPublishIn?(action: TGActionPublishIn): void|Promise<void>;

    /**
     * Subscribe type
     */
    onSubscribe?(action: TGActionSubscribe): void|Promise<void>;

    /**
     * Authenticate
     */
    onAuthenticate?(action: TGActionAuthenticate): void|Promise<void>;

    /**
     * Default action when not specific implementation is provided
     */
    default(
        action:
        |TGActionTransmit
        |TGActionInvoke
        |TGActionSubscribe
        |TGActionPublishIn
        |TGActionAuthenticate,
    ): void|Promise<void>
    {
        action.allow();
    }
}
