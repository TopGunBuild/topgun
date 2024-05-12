export interface StreamDemuxValue<T>
{
    stream?: string;
    consumerId?: number;
    data: {value: T; done: boolean};
}