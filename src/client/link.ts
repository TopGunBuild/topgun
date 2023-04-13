import {
    TGChainOptions,
    TGOnCb,
    TGMessageCb,
    TGOptionsGet,
    TGOptionsPut,
    TGValue,
} from '../types';
import { TGClient } from './client';
import { TGEvent } from './control-flow/event';
import { generateMessageId } from './graph/graph-utils';
import { TGGraph } from './graph/graph';
import { isObject } from '../utils/is-object';
import { isNotEmptyObject } from '../utils/is-empty-object';
import { isDefined } from '../utils/is-defined';
import { pubFromSoul } from '../sea';
import { match } from '../utils/match';
import { LEX } from '../types/lex';

export class TGLink 
{
    readonly key: string;
    readonly soul: string | undefined;
    optionsGet: TGOptionsGet | undefined;

    protected readonly _updateEvent: TGEvent<TGValue | undefined, string>;
    protected readonly _chain: TGClient;
    protected readonly _parent?: TGLink;
    protected _opt: TGChainOptions;
    protected _hasReceived: boolean;
    protected _lastValue: TGValue | undefined;
    protected _endQuery?: () => void;

    /* map utils */
    protected _mapLinks: { [key: string]: TGLink } | undefined;

    /**
     * Constructor
     */
    constructor(chain: TGClient, key: string, parent?: TGLink) 
    {
        this.key = key;
        this._opt = {};
        this._chain = chain;
        this._parent = parent;
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

    getGraph(): TGGraph 
    {
        return this._chain.graph;
    }

    getPath(): string[] 
    {
        if (this._parent) 
        {
            return [...this._parent.getPath(), this.key];
        }

        return [this.key];
    }

    get(key: string): TGLink 
    {
        return new (this.constructor as any)(this._chain, key, this);
    }

    back(amount = 1): TGLink | TGClient 
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

    put(value: TGValue, cb?: TGMessageCb, opt?: TGOptionsPut): TGLink 
    {
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
            this.opt().uuid,
            this._chain.pub,
            opt,
        );

        return this;
    }

    set(data: any, cb?: TGMessageCb, opt?: TGOptionsPut): TGLink 
    {
        let soul;

        if (data instanceof TGLink && data.soul) 
        {
            soul = data.soul;

            this.put(
                {
                    [soul]: {
                        '#': soul,
                    },
                },
                cb,
                opt,
            );
        }
        else if (data && data._ && data._['#']) 
        {
            soul = data && data._ && data._['#'];

            this.put(
                {
                    [soul]: data,
                },
                cb,
                opt,
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
                opt,
            );
        }
        else 
        {
            throw new Error('This data type is not supported in set()');
        }

        return this;
    }

    not(cb: (key: string) => void): TGLink 
    {
        this.promise().then(val => !isDefined(val) && cb(this.key));
        return this;
    }

    opt(options?: TGChainOptions): TGChainOptions 
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

    once(cb: TGOnCb): TGLink 
    {
        this.promise().then(val => cb(val, this.key));
        return this;
    }

    on(cb: TGOnCb): TGLink 
    {
        const callback = (val, key) => 
        {
            if (isDefined(val)) 
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

    promise(opts = { timeout: 0 }): Promise<TGValue> 
    {
        return new Promise<TGValue>((ok: (...args: any) => void) => 
        {
            const cb: TGOnCb = (val: TGValue | undefined) => 
            {
                ok(val);
                this.off(cb);
            };
            this._on(cb);

            if (opts.timeout) 
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
    // @ Protected methods
    // -----------------------------------------------------------------------------------------------------

    protected _onQueryResponse(value?: TGValue): void 
    {
        this._updateEvent.trigger(value, this.key);
        this._lastValue = value;
        this._hasReceived = true;
    }

    protected _on(cb: TGOnCb): TGLink 
    {
        this._updateEvent.on(cb);
        if (this._hasReceived) 
        {
            // TODO: Callback key or soul?
            // const soul = this._lastValue && this._lastValue._ && this._lastValue._['#'];
            cb(this._lastValue, this.key);
        }
        if (!this._endQuery) 
        {
            this._endQuery = this._chain.graph.query(
                this.getPath(),
                this._onQueryResponse.bind(this),
            );
        }
        return this;
    }

    protected _onMap(cb: TGOnCb): TGLink 
    {
        this._mapLinks = {};

        return this._on((node: TGValue | undefined) => 
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
