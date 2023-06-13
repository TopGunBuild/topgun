import { isDefined, isFunction, isNotEmptyObject, isNumber, isObject } from 'topgun-typed';
import {
    SystemEvent,
    TGChainOptions,
    TGMessageCb,
    TGOnCb,
    TGOptionsGet,
    TGOptionsPut,
    TGUserReference,
    TGValue,
} from '../types';
import { TGClient } from './client';
import { TGEvent } from './control-flow/event';
import { TGGraph } from './graph/graph';
import { pubFromSoul } from '../sea';
import { match } from '../utils/match';
import { LEX } from '../types/lex';
import { assertFn, assertNotEmptyString } from '../utils/assert';

export class TGLink
{
    key: string;
    soul: string|undefined;
    optionsGet: TGOptionsGet|undefined;

    protected readonly _updateEvent: TGEvent<TGValue|undefined, string>;
    protected readonly _chain: TGClient;
    protected readonly _parent?: TGLink;
    protected _opt: TGChainOptions;
    protected _hasReceived: boolean;
    protected _lastValue: TGValue|undefined;
    protected _endQuery?: () => void;

    /* map utils */
    protected _mapLinks: {[key: string]: TGLink}|undefined;

    /**
     * Constructor
     */
    constructor(chain: TGClient, key: string, parent?: TGLink)
    {
        this.key          = key;
        this._opt         = {};
        this._chain       = chain;
        this._parent      = parent;
        this._hasReceived = false;
        this._updateEvent = new TGEvent(this.getPath().join('|'));
        if (!parent)
        {
            this.soul = key;

            if (key.startsWith('~') && pubFromSoul(key))
            {
                this._chain.pub = pubFromSoul(key);
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    userPubExpected(): boolean
    {
        return this.getPath().some(path => path.includes(this._chain.WAIT_FOR_USER_PUB));
    }

    /**
     * Graph from the current chain link
     */
    getGraph(): TGGraph
    {
        return this._chain.graph;
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
     * @param key Key to read data from
     * @returns New chain context corresponding to given key
     */
    get(key: string): TGLink
    {
        return new (this.constructor as any)(this._chain, assertNotEmptyString(key), this);
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
            return this._chain;
        }
        if (amount === 1)
        {
            return this._parent || this._chain;
        }
        return this.back(amount - 1);
    }

    /**
     * Save data into topGun, syncing it with your connected peers.
     *
     * You do not need to re-save the entire object every time, topGun will automatically
     * merge your data into what already exists as a "partial" update.
     *
     * @param value the data to save
     * @param cb an optional callback, invoked on each acknowledgment
     * @param opt options put
     * @returns same chain context
     **/
    put(value: TGValue, cb?: TGMessageCb, opt?: TGOptionsPut): TGLink
    {
        if (this.userPubExpected())
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
        this._chain.graph.putPath(
            this.getPath(),
            value,
            cb,
            opt,
        );

        return this;
    }

    not(cb: (key: string) => void): TGLink
    {
        this.promise().then(val => !isDefined(val) && cb(this.key));
        return this;
    }

    opt(options?: TGChainOptions): TGChainOptions
    {
        if (isObject(options))
        {
            this._opt = { ...this._opt, ...options };
        }
        if (this._parent)
        {
            return { ...this._parent.opt(), ...this._opt };
        }
        return this._opt;
    }

    once(cb: TGOnCb): TGLink
    {
        cb = assertFn<TGOnCb>(cb);
        this.promise().then(val => cb(val, this.key));
        return this;
    }

    on(cb: TGOnCb): TGLink
    {
        const callback = (val, key) =>
        {
            if (isDefined(val) && isFunction(cb))
            {
                cb(val, key);
            }
        };
        return isObject(this._mapLinks)
            ? this._onMap(callback)
            : this._on(callback);
    }

    off(cb?: TGOnCb): TGLink
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

        if (isNotEmptyObject(this._mapLinks))
        {
            for (const key in this._mapLinks)
            {
                const link = this._mapLinks[key];

                if (link instanceof TGLink)
                {
                    link.off(cb);
                }
            }
        }

        return this;
    }

    promise(opts?: { timeout: number }): Promise<TGValue>
    {
        return new Promise<TGValue>((ok: (...args: any) => void) =>
        {
            const cb: TGOnCb = (val: TGValue|undefined) =>
            {
                ok(val);
                this.off(cb);
            };
            this._on(cb);

            if (isNumber(opts?.timeout))
            {
                setTimeout(() => cb(undefined), opts.timeout);
            }
        });
    }

    then(fn?: (val: TGValue) => any): Promise<any>
    {
        return this.promise().then(fn);
    }

    map(): TGLink
    {
        this._mapLinks = {};
        return this;
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
            this._chain.pub      = pub;
            this.optionsGet['#'] = this.optionsGet['#'].replace(this._chain.WAIT_FOR_USER_PUB, pub);
            this.key             = this.key.replace(this._chain.WAIT_FOR_USER_PUB, pub);
        }
    }

    private _onQueryResponse(value?: TGValue): void
    {
        this._updateEvent.trigger(value, this.key);
        this._lastValue   = value;
        this._hasReceived = true;
    }

    private _on(cb: TGOnCb): TGLink
    {
        const handler = () =>
        {
            this._updateEvent.on(cb);
            if (this._hasReceived)
            {
                // TODO: Callback key or soul?
                const soul = isObject(this._lastValue) && this._lastValue._ && this._lastValue._['#'];
                cb(this._lastValue, soul);
            }
            if (!this._endQuery)
            {
                this._endQuery = this._chain.graph.query(
                    this.getPath(),
                    this._onQueryResponse.bind(this),
                    this.optionsGet,
                );
            }
        };

        if (this.userPubExpected())
        {
            this._chain.promise<TGUserReference>(SystemEvent.auth).then((value) =>
            {
                this._setUserPub(value.pub);
                handler();
            })
        }
        else
        {
            handler();
        }

        return this;
    }

    private _onMap(cb: TGOnCb): TGLink
    {
        this._mapLinks = {};

        return this._on((node: TGValue|undefined) =>
        {
            if (isObject(node))
            {
                for (const soul in node)
                {
                    if (node.hasOwnProperty(soul) && soul !== '_')
                    {
                        // Already subscribed
                        if ((this._mapLinks as object).hasOwnProperty(soul))
                        {
                            continue;
                        }

                        // LEX condition does not pass
                        if (
                            isObject(this.optionsGet) &&
                            isNotEmptyObject(this.optionsGet['.']) &&
                            !match(soul, this.optionsGet['.'] as LEX)
                        )
                        {
                            continue;
                        }

                        // Register child listener
                        if (!(this._mapLinks as object).hasOwnProperty(soul))
                        {
                            (this._mapLinks as object)[soul] =
                                this.get(soul).on(cb);
                        }
                    }
                }
            }
        });
    }
}
