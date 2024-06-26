import { DataStream } from '@topgunbuild/data-streams';
import { isFunction } from '@topgunbuild/utils';
import { QueryCb } from '../types';
import { QueryHandler } from '../query-handlers/query-handler';

export class SelectBuilder<T>
{
    readonly #queryHandler: QueryHandler<T>;

    constructor(queryHandler: QueryHandler<T>)
    {
        this.#queryHandler = queryHandler;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    on(cb?: QueryCb<T>): DataStream<T>
    {
        return this.#handle(false, cb);
    }

    once(cb?: QueryCb<T>): DataStream<T>
    {
        return this.#handle(true, cb);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #handle(once: boolean, cb?: QueryCb<T>): DataStream<T>
    {
        this.#queryHandler.once = once;

        if (isFunction(cb))
        {
            // Get data for callback function
            (async () =>
            {
                for await (const data of this.#queryHandler.dataStream)
                {
                    cb(data);

                    // Destroy query after the result is received
                    if (once)
                    {
                        this.#queryHandler.dataStream.destroy();
                    }
                }
            })();
        }

        return this.#queryHandler.dataStream;
    }
}
