import { DataStream } from '@topgunbuild/data-streams';
import { isFunction } from '@topgunbuild/utils';
import { DataType, QueryCb } from '../types';
import { QueryHandler } from '../query-handlers/query-handler';
import { SelectOptions } from '@topgunbuild/transport';

export class SelectBuilder<D extends DataType, S extends SelectOptions>
{
    readonly #queryHandler: QueryHandler<D, S>;

    constructor(queryHandler: QueryHandler<D, S>)
    {
        this.#queryHandler = queryHandler;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    on(cb?: QueryCb<D>): DataStream<D>
    {
        return this.#handle(false, cb);
    }

    once(cb?: QueryCb<D>): DataStream<D>
    {
        return this.#handle(true, cb);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #handle(once: boolean, cb?: QueryCb<D>): DataStream<D>
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
                        // TODO: Wait for connector response
                        // await this.#queryHandler.destroy();
                    }
                }
            })();
        }

        return this.#queryHandler.dataStream;
    }
}
