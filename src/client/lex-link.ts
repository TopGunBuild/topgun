import { cloneValue, isNumber, isObject } from 'topgun-typed';
import { TGLink } from './link';
import { LEX } from '../types/lex';
import { TGClient } from './client';
import { TGData, TGOnCb, TGOptionsGet, TGValue } from '../types';
import { assertBoolean, assertNotEmptyString, assertNumber } from '../utils/assert';
import { replacerSortKeys } from '../utils/replacer-sort-keys';
import { TGStream } from '../stream/stream';

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
        this._maxLimit  = this._chain.transportMaxKeyValuePairs;
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
        this.#mergeSoul();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    start(value: string): TGLexLink
    {
        this.#setLex('>', assertNotEmptyString(value));
        return this;
    }

    end(value: string): TGLexLink
    {
        this.#setLex('<', assertNotEmptyString(value));
        return this;
    }

    prefix(value: string): TGLexLink
    {
        this.#setLex('*', assertNotEmptyString(value));
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

    once<T extends TGValue>(cb?: TGOnCb<T>): TGStream<TGData<T>>
    {
        return this._link.once(cb);
    }

    on<T extends TGValue>(cb?: TGOnCb<T>): TGStream<TGData<T>>
    {
        return this._link.on(cb);
    }

    off(): void
    {
        return this._link.off();
    }

    map(): TGLexLink
    {
        return this;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this.optionsGet['.'][key] = value;
    }

    #mergeSoul(): void
    {
        this.optionsGet['#'] = this._link.soul = this._link.getPath().join('/');
    }
}
