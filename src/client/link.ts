import { MessageCb, OptionsGet, OptionsPut, Value } from '../types'
import { Client } from './client'
import { Event } from './control-flow/event'
import { ChainOptions, OnCb } from './interfaces'
import { generateMessageId } from './graph/graph-utils';
import { Graph } from './graph/graph';
import { isObject } from '../utils/is-object';
import { isNotEmptyObject } from '../utils/is-empty-object';
import { isDefined } from '../utils/is-defined';
import { pubFromSoul } from '../sea';
import { match } from '../utils/match';


export class Link
{
    readonly key: string;
    readonly soul: string|undefined;
    optionsGet: OptionsGet|undefined;

    protected readonly _updateEvent: Event<Value|undefined, string>;
    protected readonly _chain: Client;
    protected readonly _parent?: Link;
    protected _opt: ChainOptions;
    protected _hasReceived: boolean;
    protected _lastValue: Value|undefined;
    protected _endQuery?: () => void;

    /* map utils */
    protected _mapLinks: {[key: string]: Link}|undefined;

    /**
     * Constructor
     */
    constructor(chain: Client, key: string, parent?: Link)
    {
        this.key          = key;
        this._opt         = {};
        this._chain       = chain;
        this._parent      = parent;
        this._hasReceived = false;
        this._updateEvent = new Event(this.getPath().join('|'));
        if (!parent)
        {
            this.soul = key;

            if (key.startsWith('~') && pubFromSoul(key))
            {
                this._chain.pub = pubFromSoul(key);
            }
        }
    }

    /**
     * Graph from the current chain link
     *
     * @returns {Graph}
     */
    getGraph(): Graph
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
            return [...this._parent.getPath(), this.key]
        }

        return [this.key];
    }

    /**
     * Traverse a location in the graph
     *
     * @param key Key to read data from
     * @returns New chain context corresponding to given key
     */
    get(key: string): Link
    {
        return new (this.constructor as any)(this._chain, key, this);
    }

    /**
     * Move up to the parent context on the chain.
     *
     * Every time a new chain is created, a reference to the old context is kept to go back to.
     *
     * @param amount The number of times you want to go back up the chain. {-1} or {Infinity} will take you to the root.
     * @returns a parent chain context
     */
    back(amount = 1): Link|Client
    {
        if (amount < 0 || amount === Infinity)
        {
            return this._chain
        }
        if (amount === 1)
        {
            return this._parent || this._chain
        }
        return this.back(amount - 1)
    }

    /* /!**
      * Save data into topGun, syncing it with your connected peers.
      *
      * You do not need to re-save the entire object every time, topGun will automatically
      * merge your data into what already exists as a "partial" update.
      *
      * @param value the data to save
      * @param cb an optional callback, invoked on each acknowledgment
      * @returns same chain context
      *!/*/
    /**
     *
     * @param {Value} value
     * @param {MessageCb} cb
     * @param {OptionsPut} opt
     * @returns {Link}
     */
    put(value: Value, cb?: MessageCb, opt?: OptionsPut): Link
    {
        if (!this._parent && !isObject(value))
        {
            throw new Error('Data at root of graph must be a node (an object).');
        }
        this._chain.graph.putPath(this.getPath(), value, cb, this.opt().uuid, this._chain.pub, opt);

        return this;
    }

    /**
     * Add a unique item to an unordered list.
     *
     * Works like a mathematical set, where each item in the list is unique.
     * If the item is added twice, it will be merged.
     * This means only objects, for now, are supported.
     *
     * @param data should be a topGun reference or an object
     * @param cb The callback is invoked exactly the same as .put
     * @param {OptionsPut} opt
     * @returns chain context for added object
     */
    set(data: any, cb?: MessageCb, opt?: OptionsPut): Link
    {
        let soul;

        if (data instanceof Link && data.soul)
        {
            soul = data.soul;

            this.put(
                {
                    [soul]: {
                        '#': soul
                    },
                },
                cb,
                opt
            )
        }
        else if (data && data._ && data._['#'])
        {
            soul = data && data._ && data._['#'];

            this.put(
                {
                    [soul]: data,
                },
                cb,
                opt
            );
        }
        else if (isObject(data) && isNotEmptyObject(data))
        {
            soul = generateMessageId();

            this.put(
                {
                    [soul]: data,
                },
                cb,
                opt
            );
        }
        else
        {
            throw new Error('This data type is not supported in set()');
        }

        return this
    }

    /**
     * Register a callback for when it appears a record does not exist
     *
     * If you need to know whether a property or key exists, you can check with .not.
     * It will consult the connected peers and invoke the callback if there's reasonable certainty that none of them have the data available.
     *
     * @param cb If there's reason to believe the data doesn't exist, the callback will be invoked. This can be used as a check to prevent implicitly writing data
     * @returns same chain context
     */
    not(cb: (key: string) => void): Link
    {
        this.promise().then(val => !isDefined(val) && cb(this.key));
        return this
    }

    /**
     * Change the configuration of this chain link
     *
     * @param options
     * @returns current options
     */
    opt(options?: ChainOptions): ChainOptions
    {
        if (options)
        {
            this._opt = { ...this._opt, ...options };
        }
        if (this._parent)
        {
            return { ...this._parent.opt(), ...this._opt };
        }
        return this._opt;
    }

    /**
     * Get the current data without subscribing to updates. Or undefined if it cannot be found.
     *
     * @param cb The data is the value for that chain at that given point in time. And the key is the last property name or ID of the node.
     * @returns same chain context
     */
    once(cb: OnCb): Link
    {
        this.promise().then(val => cb(val, this.key));
        return this
    }

    /**
     * Subscribe to updates and changes on a node or property in realtime.
     *
     * Triggered once initially and whenever the property or node you're focused on changes,
     * Since topGun streams data, the callback will probably be called multiple times as new chunk comes in.
     *
     * To remove a listener call .off() on the same property or node.
     *
     * @param cb The callback is immediately fired with the data as it is at that point in time.
     * @returns same chain context
     */
    on(cb: OnCb): Link
    {
        const callback = (val, key) =>
        {
            if (isDefined(val))
            {
                cb(val, key);
            }
        };
        return isObject(this._mapLinks) ? this._onMap(callback) : this._on(callback);
    }

    /**
     * Unsubscribe one or all listeners subscribed with on
     *
     * @returns same chain context
     */
    off(cb?: OnCb): Link
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
                this._endQuery()
            }
            this._updateEvent.reset()
        }

        if (isNotEmptyObject(this._mapLinks))
        {
            for (const key in this._mapLinks)
            {
                const link = this._mapLinks[key];

                if (link instanceof Link)
                {
                    link.off(cb);
                }
            }
        }

        return this
    }

    promise(opts = { timeout: 0 }): Promise<Value>
    {
        return new Promise<Value>((ok: (...args: any) => void) =>
        {
            const cb: OnCb = (val: Value|undefined) =>
            {
                ok(val);
                this.off(cb);
            };
            this._on(cb);

            if (opts.timeout)
            {
                setTimeout(() => cb(undefined), opts.timeout);
            }
        })
    }

    then(fn?: (val: Value) => any): Promise<any>
    {
        return this.promise().then(fn)
    }

    /**
     * Iterates over each property and item on a node, passing it down the chain
     *
     * Behaves like a forEach on your data.
     * It also subscribes to every item as well and listens for newly inserted items.
     *
     * @returns a new chain context holding many chains simultaneously.
     */
    map(): Link
    {
        this._mapLinks = {};
        return this;
    }

    protected _onQueryResponse(value?: Value): void
    {
        this._updateEvent.trigger(value, this.key);
        this._lastValue   = value;
        this._hasReceived = true
    }

    protected _on(cb: OnCb): Link
    {
        this._updateEvent.on(cb);
        if (this._hasReceived)
        {
            // TODO: Callback key or soul?
            // const soul = this._lastValue && this._lastValue._ && this._lastValue._['#'];
            cb(this._lastValue, this.key)
        }
        if (!this._endQuery)
        {
            this._endQuery = this._chain.graph.query(
                this.getPath(),
                this._onQueryResponse.bind(this)
            );
        }
        return this
    }

    protected _onMap(cb: OnCb): Link
    {
        this._mapLinks = {};

        return this._on((node: Node|undefined) =>
        {
            if (isObject(node))
            {
                for (const soul in node)
                {
                    if (node.hasOwnProperty(soul) && soul !== '_')
                    {
                        // Already subscribed
                        if (this._mapLinks.hasOwnProperty(soul))
                        {
                            continue;
                        }

                        // LEX condition does not pass
                        if (
                            isObject(this.optionsGet) &&
                            isNotEmptyObject(this.optionsGet['.']) &&
                            !match(soul, this.optionsGet['.'])
                        )
                        {
                            continue;
                        }

                        // Register child listener
                        if (!this._mapLinks.hasOwnProperty(soul))
                        {
                            this._mapLinks[soul] = this.get(soul).on(cb);
                        }
                    }
                }
            }
        });
    }
}
