import { cloneValue, isEmptyObject, isFunction, isString, isNumber, isObject } from 'topgun-typed';
import {
    SystemEvent,
    TGGraphData, TGMessage,
    TGMessageCb,
    TGOnCb,
    TGOptionsGet,
    TGOptionsPut,
    TGValue,
} from '../types';
import { TGClient } from './client';
import { TGEvent } from './control-flow/event';
import { TGGraph } from './graph/graph';
import { pubFromSoul } from '../sea';
import { assertFn, assertNotEmptyString, assertGetPath } from '../utils/assert';
import { getNodeSoul, isNode } from '../utils/node';
import { generateMessageId } from './graph/graph-utils';
import { TGLexLink } from './lex-link';

/* eslint-disable @typescript-eslint/no-empty-function */
export class TGLink
{
    key: string;
    soul: string|undefined;
    _lex?: TGLexLink;

    protected readonly _updateEvent: TGEvent<TGValue|undefined, string>;
    protected readonly _client: TGClient;
    protected readonly _parent?: TGLink;
    protected _hasReceived: boolean;
    protected _lastValue: TGValue|undefined;
    protected _endQuery?: () => void;

    /**
     * Constructor
     */
    constructor(client: TGClient, key: string, parent?: TGLink)
    {
        this.key          = key;
        this._client      = client;
        this._parent      = parent;
        this._hasReceived = false;
        this._updateEvent = new TGEvent(this.getPath().join('|'));
        if (!parent)
        {
            this.soul = key;

            if (key.startsWith('~') && pubFromSoul(key))
            {
                this._client.pub = pubFromSoul(key);
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    waitForAuth(): boolean
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
            return new TGLexLink(this._client, keyOrOptions, this);
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
        return this.back(amount - 1);
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
        return new Promise<TGMessage>((resolve, reject) =>
        {
            let soulSuffix, value = cloneValue(data);

            if (!isObject(value) || isEmptyObject(value))
            {
                throw new Error('This data type is not supported in set().');
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
                soulSuffix = generateMessageId();
            }

            this.key = this.key.endsWith('/') ? `${this.key}${soulSuffix}` : `${this.key}/${soulSuffix}`;

            return this.put(value, cb, opt).then(resolve).catch(reject);
        });
    }

    once(cb: TGOnCb, timeout = 500): Promise<TGValue|undefined>
    {
        cb = assertFn<TGOnCb>(cb);
        return this.promise({ timeout, cb });
    }

    on(cb: TGOnCb): void
    {
        cb = assertFn<TGOnCb>(cb);
        if (this._lex)
        {
            this._onMap(cb);
        }
        else
        {
            this._on(cb);
        }
    }

    off(cb?: TGOnCb): void
    {
        if (cb)
        {
            this._updateEvent.off(cb);
            if (this._endQuery && this._updateEvent.listenerCount() === 0)
            {
                this._endQuery();
            }
        }
        else
        {
            if (this._endQuery)
            {
                this._endQuery();
            }
            this._updateEvent.reset();
        }
    }

    promise(opts?: {timeout: number, cb?: TGOnCb}): Promise<TGValue|undefined>
    {
        return new Promise<TGValue>((ok: (...args: any) => void) =>
        {
            const connectorMsgId    = generateMessageId();
            const connectorCallback = (data?: TGGraphData, msgId?: string) => connectorMsgId === msgId && resolve();
            const resolve           = (val?: TGValue) =>
            {
                ok(val);
                this.off(callback);
                this._client.graph.events.graphData.off(connectorCallback);
            };
            const originalCallback  = isFunction(opts && opts.cb) ? opts.cb : () =>
            {
            };
            const callback          = (val: TGValue|undefined, soul?: string) =>
            {
                originalCallback(val, soul);

                // Terminate if only one node is requested
                if (!this._lex)
                {
                    resolve(val);
                }
            };

            if (this._lex)
            {
                this._onMap(callback, connectorMsgId);

                if (this._client.graph.activeConnectors > 0)
                {
                    // Wait until at least one of the connectors returns a graph data
                    this._client.graph.events.graphData.on(connectorCallback);
                }
                else
                {
                    resolve();
                }
            }
            else
            {
                this._on(callback);
            }

            // Resolve by timeout
            if (isNumber(opts?.timeout))
            {
                setTimeout(() => resolve(), opts.timeout);
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

            if (isObject(this._lex.optionsGet) && isString(this._lex.optionsGet['#']))
            {
                this._lex.optionsGet['#'] = this._lex.optionsGet['#'].replace(this._client.WAIT_FOR_USER_PUB, pub);
            }
        }
    }

    private _onQueryResponse(value?: TGValue): void
    {
        const soul = getNodeSoul(value) || this.key;
        this._updateEvent.trigger(value, soul);
        this._lastValue   = value;
        this._hasReceived = true;
    }

    private _on(cb: TGOnCb): TGLink
    {
        return this._maybeWaitAuth(() =>
        {
            this._updateEvent.on(cb);
            if (this._hasReceived)
            {
                cb(this._lastValue, this.key);
            }
            if (!this._endQuery)
            {
                this._endQuery = this._client.graph.query(
                    this.getPath(),
                    this._onQueryResponse.bind(this),
                );
            }
        });
    }

    private _onMap(cb: TGOnCb, msgId?: string): TGLink
    {
        return this._maybeWaitAuth(() =>
        {
            this._updateEvent.on(cb);
            if (!this._endQuery)
            {
                this._endQuery = this._client.graph.queryMany(
                    this._lex.optionsGet,
                    this._onQueryResponse.bind(this),
                    msgId
                );
            }
        });
    }

    private _maybeWaitAuth(handler: () => void): TGLink
    {
        if (this.waitForAuth())
        {
            this._client.listener(SystemEvent.Auth).once().then((value) =>
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
