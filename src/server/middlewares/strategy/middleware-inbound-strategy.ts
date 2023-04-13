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
     * @param action
     */
    onTransmit?(action: TGActionTransmit): void | Promise<void>;

    /**
     * Invoke type
     * @param action
     */
    onInvoke?(action: TGActionInvoke): void | Promise<void>;

    /**
     * Publish_in type
     * @param action
     */
    onPublishIn?(action: TGActionPublishIn): void | Promise<void>;

    /**
     * Subscribe type
     * @param action
     */
    onSubscribe?(action: TGActionSubscribe): void | Promise<void>;

    /**
     * Authenticate
     * @param action
     */
    onAuthenticate?(action: TGActionAuthenticate): void | Promise<void>;

    /**
     * Default action when not specific implementation is provided
     * @param action
     */
    default(
        action:
            | TGActionTransmit
            | TGActionInvoke
            | TGActionSubscribe
            | TGActionPublishIn
            | TGActionAuthenticate,
    ): void | Promise<void> 
    {
        action.allow();
    }
}
