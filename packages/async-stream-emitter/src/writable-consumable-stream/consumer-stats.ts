export interface ConsumerStats
{
    id: number;
    backpressure: number;
    timeout?: number | undefined;
    stream?: any;
}
