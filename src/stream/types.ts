export type TGStreamState = 'pending'|'subscribed'|'unsubscribed'|'destroyed';

export interface TGSimpleStream
{
    name: string;
    state: TGStreamState;
    attributes: {[key: string]: any};
}
