import { cloneValue, isEmptyObject, isNumber, isObject, isString, isFunction } from 'topgun-typed';
import {
    TGData,
    TGMessage,
    TGMessageCb,
    TGOnCb,
    TGOptionsGet,
    TGOptionsPut,
    TGValue,
} from '../types';
import { TGClient } from './client';
import { TGGraph } from './graph/graph';
import { pubFromSoul } from '../sea';
import { assertGetPath, assertNotEmptyString, assertOptionsGet } from '../utils/assert';
import { getNodeSoul, isNode } from '../utils/node';
import { TGLexLink } from './lex-link';
import { uuidv4 } from '../utils/uuidv4';
import { TGStream } from '../stream/stream';
import { TGExchange } from '../stream/exchange';

/* eslint-disable @typescript-eslint/no-empty-function */
export class TGLink
{
    readonly id: string;
    key: string;
    soul: string|undefined;
    _lex?: TGLexLink;

    protected readonly _client: TGClient;
    protected readonly _parent?: TGLink;
    protected readonly _exchange: TGExchange;
    protected _receivedData: {
        [streamName: string]: {
            [soul: string]: TGValue
        }
    };
    protected _endQueries?: {
        [streamName: string]: () => void
    };

    /**
     * Constructor
     */
    constructor(client: TGClient, key: string, parent?: TGLink)
    {
        this.id            = uuidv4();
        this.key           = key;
        this._client       = client;
        this._parent       = parent;
        this._exchange     = new TGExchange();
        this._receivedData = {};
        this._endQueries   = {};
        if (!parent)
        {
            this.soul = key;

            if (key.startsWith('~') && pubFromSoul(key))
            {
                this._client.pub = pubFromSoul(key);
            }
        }

        (async () =>
        {
            for await (const { streamName } of this._exchange.listener('destroy'))
            {
                if (isFunction(this._endQueries[streamName]))
                {
                    this._endQueries[streamName]();
                }
            }
        })();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    multiQuery(): boolean
    {
        return this._lex instanceof TGLexLink;
    }

    waitForAuth(): boolean
    {
        if (this._client.user().is?.pub)
        {
            if (this.userPubExpected())
            {
                this._setUserPub(this._client.user().is?.pub);
            }
            return false;
        }
        return this.userPubExpected();
    }

    userPubExpected(): boolean
    {
        return this.getPath().some(path => path.includes(this._client.WAIT_FOR_USER_PUB));
    }

    /**
     * Graph from the current chain link
     */
    getGraph(): TGGraph
    {
        return this._client.graph;
    }

    /**
     * @returns path of this node
     */
    getPath(): string[]
    {
        if (this._parent)
        {
            return [...this._parent.getPath(), this.key];
        }

        return [this.key];
    }

    /**
     * Traverse a location in the graph
     *
     * @param query Key to read data from or LEX query
     * @returns New chain context corresponding to given key
     */
    get(query: TGOptionsGet): TGLexLink;
    get(key: string): TGLink;
    get(keyOrOptions: string|TGOptionsGet): TGLink|TGLexLink
    {
        // The argument is a LEX query
        if (isObject(keyOrOptions))
        {
            return new TGLexLink(this._client, assertOptionsGet(keyOrOptions), this);
        }
        else if (isString(keyOrOptions))
        {
            return new TGLink(this._client, assertGetPath(keyOrOptions), this);
        }
        else
        {
            throw Error('Get path must be string or query object.');
        }
    }

    /**
     * Move up to the parent context on the chain.
     *
     * Every time a new chain is created, a reference to the old context is kept to go back to.
     *
     * @param amount The number of times you want to go back up the chain. {-1} or {Infinity} will take you to the root.
     * @returns a parent chain context
     */
    back(amount = 1): TGLink|TGClient
    {
        if (amount < 0 || amount === Infinity)
        {
            return this._client;
        }
        if (amount === 1)
        {
            return this._parent || this._client;
        }
        return this._parent.back(amount - 1);
    }

    /**
     * Save data into topGun, syncing it with your connected peers.
     *
     * You do not need to re-save the entire object every time, TopGun will automatically
     * merge your data into what already exists as a "partial" update.
     **/
    put(value: TGValue, cb?: TGMessageCb, opt?: TGOptionsPut): Promise<TGMessage>
    {
        return new Promise<TGMessage>((resolve) =>
        {
            if (this.waitForAuth())
            {
                throw new Error(
                    'You cannot save data to user space if the user is not authorized.',
                );
            }
            if (!this._parent && !isObject(value))
            {
                throw new Error(
                    'Data at root of graph must be a node (an object).',
                );
            }

            const callback = (msg: TGMessage) =>
            {
                if (cb)
                {
                    cb(msg);
                }
                resolve(msg);
            };

            this._client.graph.putPath(
                this.getPath(),
                value,
                callback,
                opt,
            );
        })
    }

    set(data: any, cb?: TGMessageCb, opt?: TGOptionsPut): Promise<TGMessage>
    {
        return new Promise<TGMessage>((resolve) =>
        {
            let soulSuffix, value = cloneValue(data);

            if (!isObject(value) || isEmptyObject(value))
            {
                throw new Error('This data type is not supported in set().');
            }
            else if (this.waitForAuth())
            {
                throw new Error(
                    'You cannot save data to user space if the user is not authorized.',
                );
            }

            if (data instanceof TGLink)
            {
                if (data.getPath().length === 0)
                {
                    throw new Error('Link is empty.');
                }

                soulSuffix = assertNotEmptyString(data.getPath()[0]);
                value      = { '#': soulSuffix };
            }
            else if (isNode(data))
            {
                soulSuffix = assertNotEmptyString(data._['#']);
            }
            else
            {
                soulSuffix = uuidv4();
            }

            const callback = (msg: TGMessage) =>
            {
                if (cb)
                {
                    cb(msg);
                }
                resolve(msg);
            };

            const pathArr               = this.getPath();
            pathArr[pathArr.length - 1] = [this.key, soulSuffix].join('/');

            this._client.graph.putPath(
                pathArr,
                value,
                callback,
                opt,
            );
        });
    }

    once<T>(cb?: TGOnCb<T>): TGStream<TGData<T>>
    {
        const stream = this._exchange.subscribe<TGData<T>>(uuidv4(), { once: true });

        if (isFunction(cb))
        {
            (async () =>
            {
                for await (const { value, key } of stream)
                {
                    cb(value, key);

                    if (!this.multiQuery())
                    {
                        stream.destroy();
                    }
                }
            })();
        }

        return this.multiQuery() ? this._onMap(stream) : this._on(stream);
    }

    on<T>(cb?: TGOnCb<T>): TGStream<TGData<T>>
    {
        const stream = this._exchange.subscribe<TGData<T>>();

        if (isFunction(cb))
        {
            (async () =>
            {
                for await (const { value, key } of stream)
                {
                    cb(value, key);
                }
            })();
        }

        return this.multiQuery() ? this._onMap(stream) : this._on(stream);
    }

    off(): void
    {
        this._exchange.destroy();
        this._receivedData = {};
        this._endQueries   = {};
    }

    promise<T>(opts?: {timeout?: number}): Promise<T>
    {
        return new Promise<T>((resolve, reject) =>
        {
            if (this.multiQuery())
            {
                return reject(Error('For multiple use once() or on() method'));
            }

            const stream = this._exchange.subscribe<TGData<T>>();

            (async () =>
            {
                for await (const { value } of this._on(stream))
                {
                    resolve(value);
                    stream.destroy();
                }
            })();

            // Set termination timeout if there are no connectors
            if (this._client.connectors().length === 0 && !isNumber(opts?.timeout))
            {
                if (!isObject(opts))
                {
                    opts = {};
                }
                opts.timeout = 50;
            }

            if (isNumber(opts?.timeout))
            {
                setTimeout(() =>
                {
                    if (stream.state !== 'destroyed')
                    {
                        stream.destroy();
                        resolve(undefined);
                    }
                }, opts.timeout);
            }
        });
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods for collections
    // -----------------------------------------------------------------------------------------------------

    map(): TGLexLink
    {
        return new TGLexLink(this._client, {}, this);
    }

    start(value: string): TGLexLink
    {
        return this.map().start(value);
    }

    end(value: string): TGLexLink
    {
        return this.map().end(value);
    }

    prefix(value: string): TGLexLink
    {
        return this.map().prefix(value);
    }

    limit(value: number): TGLexLink
    {
        return this.map().limit(value);
    }

    reverse(value = true): TGLexLink
    {
        return this.map().reverse(value);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _setUserPub(pub: string): void
    {
        if (this._parent)
        {
            this._parent._setUserPub(pub);
        }
        else
        {
            this._client.pub = pub;
            this.key         = this.key.replace(this._client.WAIT_FOR_USER_PUB, pub);

            if (
                this._lex instanceof TGLexLink &&
                isObject(this._lex.optionsGet) &&
                isString(this._lex.optionsGet['#'])
            )
            {
                this._lex.optionsGet['#'] = this._lex.optionsGet['#'].replace(this._client.WAIT_FOR_USER_PUB, pub);
            }
        }
    }

    private _onQueryResponse<T>(value: TGValue, stream: TGStream<T>): void
    {
        const key = getNodeSoul(value) || this.key;

        if (stream.attributes['once'])
        {
            if (!isObject(this._receivedData[stream.name]))
            {
                this._receivedData[stream.name] = {};
            }
            if (this._receivedData[stream.name][key])
            {
                return;
            }
            this._receivedData[stream.name][key] = value;
        }

        stream.publish({ value, key });
    }

    private _on<T>(stream: TGStream<T>): TGStream<T>
    {
        this._maybeWaitAuth(() =>
        {
            this._endQueries[stream.name] = this._client.graph.query(
                this.getPath(),
                (value: TGValue) => this._onQueryResponse(value, stream),
                stream.name
            );
        });

        return stream;
    }

    private _onMap<T extends TGValue>(stream: TGStream<T>): TGStream<T>
    {
        this._maybeWaitAuth(() =>
        {
            this._endQueries[stream.name] = this._client.graph.queryMany(
                this._lex.optionsGet,
                (value: TGValue) => this._onQueryResponse(value, stream),
                stream.name
            );
        });

        return stream;
    }

    private _maybeWaitAuth(handler: () => void): TGLink
    {
        if (this.waitForAuth())
        {
            this._client.listener('auth').once().then((value) =>
            {
                this._setUserPub(value.pub);
                handler();
            });
        }
        else
        {
            handler();
        }

        return this;
    }
}
