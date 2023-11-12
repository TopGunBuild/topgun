import { cloneValue, isEmptyObject, isNumber, isObject, isString, isFunction } from '@topgunbuild/typed';
import {
    TGCollectionOptions,
    TGData,
    TGMessage,
    TGOnCb,
    TGOptionsGet,
    TGOptionsPut,
    TGValue,
} from '../../types';
import { TGClient } from '../client';
import { TGGraph } from '../graph/graph';
import { pubFromSoul } from '../../sea';
import { assertGetPath, assertOptionsGet } from '../../utils/assert';
import { getNodeSoul, isNode } from '../../utils/node';
import { TGLexLink } from './lex-link';
import { uuidv4 } from '../../utils/uuidv4';
import { TGStream } from '../../stream/stream';
import { TGExchange } from '../../stream/exchange';
import { createSoul, isMessage } from '../../utils';

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

            // If this is user space
            if (key.startsWith('~') && pubFromSoul(key))
            {
                this._client.pub = pubFromSoul(key);
            }
        }

        (async () =>
        {
            // Unsubscribe from requests when link is destroyed
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

    /**
     * True, if link requires user authorization
     */
    authRequired(): boolean
    {
        if (this._client.user().is?.pub)
        {
            if (this.userPubExpected())
            {
                this.__setUserPub(this._client.user().is?.pub);
            }
            return false;
        }
        return this.userPubExpected();
    }

    /**
     * True, if the user’s public key has not yet been recorded in the path
     */
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

    getSoul(): string
    {
        return createSoul(
            ...this.getPath()
        );
    }

    /**
     * Traverse a location in the graph
     */
    get(query: TGOptionsGet): TGLexLink;
    get(key: string): TGLink;
    get(keyOrOptions: string|TGOptionsGet): TGLink|TGLexLink
    {
        // The argument is a LEX query
        if (isObject(keyOrOptions))
        {
            return new TGLexLink(this, this._client.options.transportMaxKeyValuePairs, assertOptionsGet(keyOrOptions));
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
     * Set null data instead of node at this path
     */
    async remove(opt?: TGOptionsPut): Promise<void>
    {
        const cb = () =>
        {
        };

        await this._client.graph.putPath(
            [
                this.getPath().join('/')
            ],
            null,
            cb,
            opt
        );
    }

    /**
     * Save data into topGun, syncing it with your connected peers.
     *
     * You do not need to re-save the entire object every time, TopGun will automatically
     * merge your data into what already exists as a "partial" update.
     **/
    put(value: TGValue|TGLink, opt?: TGOptionsPut): Promise<TGMessage>
    {
        return new Promise<TGMessage>((resolve) =>
        {
            if (this.authRequired())
            {
                throw new Error(
                    'You cannot save data to user space if the user is not authorized.',
                );
            }
            else if (!this._parent && (!isObject(value) && value !== null))
            {
                throw new Error(
                    'Data at root of graph must be a node (an object) or null.',
                );
            }
            else if (value instanceof TGLink)
            {
                if (value.getPath().length > 2)
                {
                    throw new Error('Data at root of graph must be a node (an object) or null.');
                }
                value = { '#': value.getPath().join('/') };
            }
            else if (isMessage(value))
            {
                value = { '#': value['#'] };
            }

            this._client.graph.putPath(
                this.getPath(),
                value,
                resolve,
                opt,
            );
        })
    }

    /**
     * Add a unique item to an unordered list
     */
    set(data: TGValue, opt?: TGOptionsPut): Promise<TGMessage>
    {
        return new Promise<TGMessage>((resolve) =>
        {
            let value = cloneValue(data);

            if (!isObject(value) || isEmptyObject(value))
            {
                throw new Error('This data type is not supported in set().');
            }
            else if (this.authRequired())
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

                value = { '#': data.getSoul() };
            }
            else if (isMessage(data))
            {
                value = { '#': data['#'] };
            }
            else if (isNode(data))
            {
                delete data['_'];
            }

            const pathArr = [
                createSoul(...this.getPath(), uuidv4())
            ];

            this._client.graph.putPath(
                pathArr,
                value,
                resolve,
                opt,
            );
        });
    }

    /**
     * Get the current data without subscribing to updates
     */
    once<T extends TGValue>(cb?: TGOnCb<T>): TGStream<TGData<T>>
    {
        const stream = this.#createQueryStream<T>({
            once            : true,
            topGunCollection: this.#collectionQuery()
        });

        if (isFunction(cb))
        {
            // Get data for callback function
            (async () =>
            {
                for await (const { value, key } of stream)
                {
                    cb(value, key);

                    // Destroy query for one element after the result is received
                    if (!this.#collectionQuery())
                    {
                        stream.destroy();
                    }
                }
            })();
        }

        return this.#collectionQuery() ? this.#onMap(stream) : this.#on(stream);
    }

    /**
     * Subscribe to updates and changes on a node or property in realtime
     */
    on<T extends TGValue>(cb?: TGOnCb<T>): TGStream<TGData<T>>
    {
        const stream = this.#createQueryStream<T>({
            topGunCollection: this.#collectionQuery()
        });

        if (isFunction(cb))
        {
            // Get data for callback function
            (async () =>
            {
                for await (const { value, key } of stream)
                {
                    cb(value, key);
                }
            })();
        }

        return this.#collectionQuery() ? this.#onMap(stream) : this.#on(stream);
    }

    /**
     * Destroy all queries
     */
    off(): void
    {
        this._exchange.destroy();
        this._receivedData = {};
        this._endQueries   = {};
    }

    /**
     * Get current data once as a promise
     */
    promise<T extends TGValue>(opts?: {timeout?: number}): Promise<T>
    {
        return new Promise<T>((resolve, reject) =>
        {
            if (this.#collectionQuery())
            {
                return reject(Error('For multiple use once() or on() method'));
            }

            const stream = this.#createQueryStream<T>({ once: true });

            (async () =>
            {
                for await (const { value } of this.#on(stream))
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

            // End after timeout
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

    collection(collectionOptions: TGCollectionOptions = {}): TGLexLink
    {
        return new TGLexLink(this, this._client.options.transportMaxKeyValuePairs).collection(collectionOptions);
    }

    start(value: string): TGLexLink
    {
        return this.collection().start(value);
    }

    end(value: string): TGLexLink
    {
        return this.collection().end(value);
    }

    prefix(value: string): TGLexLink
    {
        return this.collection().prefix(value);
    }

    limit(value: number): TGLexLink
    {
        return this.collection().limit(value);
    }

    reverse(value = true): TGLexLink
    {
        return this.collection().reverse(value);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    __setUserPub(pub: string): void
    {
        if (this._parent)
        {
            this._parent.__setUserPub(pub);
        }

        this._client.pub = pub;
        this.key         = this.key.replace(this._client.WAIT_FOR_USER_PUB, pub);

        if (
            this._lex instanceof TGLexLink &&
            isObject(this._lex.options) &&
            isString(this._lex.options['#'])
        )
        {
            this._lex.options['#'] = this._lex.options['#'].replace(this._client.WAIT_FOR_USER_PUB, pub);
        }
    }

    #createQueryStream<T>(attributes?: {[key: string]: any}): TGStream<TGData<T>>
    {
        return this._exchange.subscribe<TGData<T>>(uuidv4(), attributes);
    }

    #collectionQuery(): TGCollectionOptions
    {
        return this._lex instanceof TGLexLink && this._lex.collectionOptions;
    }

    #onQueryResponse<T extends TGValue>(value: T, soul: string, stream: TGStream<TGData<T>>): void
    {
        const key = soul || getNodeSoul(value) || this.key;

        // Handle once query
        // Break when data for the current key has already been received
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

    #on<T extends TGValue>(stream: TGStream<TGData<T>>): TGStream<TGData<T>>
    {
        this.#maybeWaitAuth(() =>
        {
            this._endQueries[stream.name] = this._client.graph.query(
                this.getPath(),
                (value: TGValue, soul: string) => this.#onQueryResponse(value, soul, stream),
                stream.name,
                !!stream.attributes['once']
            );
        });

        return stream;
    }

    #onMap<T extends TGValue>(stream: TGStream<TGData<T>>): TGStream<TGData<T>>
    {
        this.#maybeWaitAuth(() =>
        {
            this._endQueries[stream.name] = this._client.graph.queryMany(
                this._lex.options,
                (value: TGValue, soul: string) => this.#onQueryResponse(value, soul, stream),
                stream.name,
                !!stream.attributes['once']
            );
        });

        return stream;
    }

    #maybeWaitAuth(handler: () => void): TGLink
    {
        if (this.authRequired())
        {
            this._client.listener('auth').once().then((value) =>
            {
                this.__setUserPub(value.pub);
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
