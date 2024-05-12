export interface ConsumableStreamConsumer<T> {
    next(): Promise<IteratorResult<T>>;

    return(value?: any): any;
}

/* eslint-disable @typescript-eslint/no-empty-function */
/* eslint-disable @typescript-eslint/no-unused-vars */
export abstract class ConsumableStream<T> implements AsyncIterator<T>, AsyncIterable<T>
{
    async next(timeout?: number): Promise<IteratorResult<T>>
    {
        const asyncIterator = this.createConsumer(timeout);
        const result = await asyncIterator.next();
        (asyncIterator as AsyncIterator<any>).return();
        return result;
    }

    async once(timeout?: number): Promise<T>
    {
        const result = await this.next(timeout);
        if (result.done)
        {
            // If stream was ended, this function should never resolve.
            await new Promise(() =>
            {});
        }
        return result.value;
    }

    createConsumer(timeout?: number): ConsumableStreamConsumer<T>
    {
        throw new TypeError('Method must be overriden by subclass');
    }

    [Symbol.asyncIterator](): AsyncIterator<T>
    {
        return this.createConsumer();
    }
}
