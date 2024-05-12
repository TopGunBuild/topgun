export type ChannelState = 'pending' | 'subscribed' | 'unsubscribed';

export interface ChannelOptions
{
    waitForAuth?: boolean | undefined;
    batch?: boolean | undefined;
    data?: any;
    channel?: string;
}
