import { cloneValue, isNumber, isString, isObject } from 'topgun-typed';
import { TGLink } from './link';
import { LEX } from '../types/lex';
import { TGClient } from './client';
import { TGOptionsGet, TGValue } from '../types';

type KeyOfLex = keyof LEX;
type ValueOfLex = LEX[KeyOfLex];

export class TGLexLink extends TGLink
{
    maxLimit: number;

    /**
     * Constructor
     */
    constructor(chain: TGClient, key: string)
    {
        super(chain, key);
        this.maxLimit = this._chain.options.transportMaxKeyValuePairs;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Where to read data from
     *
     * @param query Key to read data from or LEX query
     * @returns New chain context corresponding to given key
     */
    get(query: TGOptionsGet): TGLexLink;
    get(key: string): TGLink;
    get(keyOrOptions: string|TGOptionsGet): TGLink|TGLexLink
    {
        // The argument is a LEX query
        if (isObject(keyOrOptions) && !isString(keyOrOptions))
        {
            this._persistOptions();

            if (isObject(keyOrOptions['.']))
            {
                this.optionsGet['.'] = cloneValue(keyOrOptions['.']);
            }
            if (isNumber(keyOrOptions['%']))
            {
                this.optionsGet['%'] = keyOrOptions['%'];
            }
            return this;
        }
        else
        {
            return new TGLink(this._chain, keyOrOptions as string, this);
        }
    }

    start(value: string): TGLexLink
    {
        this._setLex('>', value);
        return this;
    }

    end(value: string): TGLexLink
    {
        this._setLex('<', value);
        return this;
    }

    prefix(value: string): TGLexLink
    {
        this._setLex('*', value);
        return this;
    }

    equals(value: string): TGLexLink
    {
        this._setLex('=', value);
        return this;
    }

    limit(value: number): TGLexLink
    {
        if (value > this.maxLimit)
        {
            throw Error(
                `Limit exceeds the maximum allowed. The maximum length is ${this.maxLimit}`
            );
        }
        (this.optionsGet as object)['%'] = value;
        return this;
    }

    reverse(value = true): TGLexLink
    {
        (this.optionsGet as object)['-'] = value;
        return this;
    }

    map(): TGLink
    {
        return super.map();
    }

    toString(): string
    {
        return JSON.stringify(this.optionsGet);
    }

    getQuery(): TGOptionsGet
    {
        return this.optionsGet as TGOptionsGet;
    }

    on(cb: (node: TGValue|undefined, key?: string) => void): TGLink
    {
        return super.on(cb);
    }

    once(cb: (node: TGValue|undefined, key?: string) => void): TGLink
    {
        return super.once(cb);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this._persistOptions();
        (this.optionsGet as object)['.'][key] = value;
    }

    private _persistOptions(): void
    {
        if (!isObject(this.optionsGet))
        {
            const soul      = this.getPath().shift();
            this.optionsGet = {
                ['#']: soul,
                ['.']: {},
                ['%']: this.maxLimit
            };
        }
    }
}
