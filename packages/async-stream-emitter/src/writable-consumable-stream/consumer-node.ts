export interface ConsumerNode<T>
{
    consumerId?: number;
    next: ConsumerNode<T>|null;
    data: {
        value: T;
        done: boolean;
    };
}