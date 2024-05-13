export type ChannelState = 'pending' | 'subscribed' | 'unsubscribed';

export interface ChannelOptions
{
    batch?: boolean | undefined;
    data?: any;
    channel?: string;
}
