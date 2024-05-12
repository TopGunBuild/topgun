import { ChannelState } from '../channel/types';

export interface SimpleChannel
{
    name: string;
    state: ChannelState;
}

export interface SimpleSocket
{
    id: string;

    transmit?(event: string, data: any, options?: any): Promise<void>
}

export interface SubscribeData
{
    channel: string;
}

export interface UnsubscribeData
{
    channel: string;
}

export interface ErrorData
{
    error: any;
}

export interface PublishData
{
    channel: string;
    data: any;
}