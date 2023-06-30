import { cloneValue, isNumber, isObject } from 'topgun-typed';
import { DemuxedConsumableStream } from 'topgun-async-stream-emitter';
import { TGLink } from './link';
import { LEX } from '../types/lex';
import { TGClient } from './client';
import { TGData, TGOnCb, TGOptionsGet } from '../types';
import { assertBoolean, assertNotEmptyString, assertNumber } from '../utils/assert';
import { replacerSortKeys } from '../utils/replacer-sort-keys';

type KeyOfLex = keyof LEX;
type ValueOfLex = LEX[KeyOfLex];

export class TGLexLink
{
    readonly optionsGet: TGOptionsGet;

    private readonly _maxLimit: number;
    private readonly _link: TGLink;
    private readonly _chain: TGClient;

    /**
     * Constructor
     */
    constructor(chain: TGClient, optionsGet: TGOptionsGet, link: TGLink)
    {
        this._chain     = chain;
        this._link      = link;
        this._link._lex = this;
        this._maxLimit  = this._chain.options.transportMaxKeyValuePairs;
        this.optionsGet = {
            '.': {},
            '%': this._maxLimit
        };
        if (isObject(optionsGet))
        {
            if (isObject(optionsGet['.']))
            {
                this.optionsGet['.'] = cloneValue(optionsGet['.']);
            }
            if (isNumber(optionsGet['%']))
            {
                this.optionsGet['%'] = optionsGet['%'];
            }
        }
        this._mergeSoul();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    start(value: string): TGLexLink
    {
        this._setLex('>', assertNotEmptyString(value));
        return this;
    }

    end(value: string): TGLexLink
    {
        this._setLex('<', assertNotEmptyString(value));
        return this;
    }

    prefix(value: string): TGLexLink
    {
        this._setLex('*', assertNotEmptyString(value));
        return this;
    }

    limit(value: number): TGLexLink
    {
        if (value > this._maxLimit)
        {
            throw Error(
                `Limit exceeds the maximum allowed. The maximum length is ${this._maxLimit}`
            );
        }
        this.optionsGet['%'] = assertNumber(value);
        return this;
    }

    reverse(value = true): TGLexLink
    {
        this.optionsGet['-'] = assertBoolean(value);
        return this;
    }

    toString(): string
    {
        return JSON.stringify(this.optionsGet, replacerSortKeys);
    }

    getQuery(): TGOptionsGet
    {
        return this.optionsGet;
    }

    once(cb: TGOnCb): TGLink
    {
        return this._link.once(cb);
    }

    on(cb: TGOnCb): TGLink
    {
        return this._link.on(cb);
    }

    stream(): DemuxedConsumableStream<TGData>
    {
        return this._link.stream();
    }

    off(cb?: TGOnCb): void
    {
        return this._link.off(cb);
    }

    map(): TGLexLink
    {
        return this;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this.optionsGet['.'][key] = value;
    }

    private _mergeSoul(): void
    {
        this.optionsGet['#'] = this._link.soul = this._link.getPath().join();
    }
}
