export type DataStreamState = 'pending'|'subscribed'|'unsubscribed'|'destroyed';

export interface SimpleDataStream
{
    name: string;
    state: DataStreamState;
    attributes: {[key: string]: any};
}
