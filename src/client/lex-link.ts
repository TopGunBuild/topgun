import { cloneValue, isEmptyObject, isNumber, isObject, isString } from 'topgun-typed';
import { TGLink } from './link';
import { LEX } from '../types/lex';
import { TGClient } from './client';
import { TGMessageCb, TGOptionsGet, TGOptionsPut } from '../types';
import { assertBoolean, assertNotEmptyString, assertNumber } from '../utils/assert';
import { generateMessageId } from './graph/graph-utils';
import { isNode } from '../utils/node';
import { replacerSortKeys } from '../utils/replacer-sort-keys';

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
        this.maxLimit   = this._chain.options.transportMaxKeyValuePairs;
        this.optionsGet = {
            '.': {},
            '%': this.maxLimit
        };
        this._mergeSoul();
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
        if (isObject(keyOrOptions))
        {
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
        else if (isString(keyOrOptions))
        {
            return new TGLink(this._chain, keyOrOptions as string, this);
        }
        else
        {
            throw Error('Get path must be string or query object.');
        }
    }

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
        if (value > this.maxLimit)
        {
            throw Error(
                `Limit exceeds the maximum allowed. The maximum length is ${this.maxLimit}`
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
        return JSON.stringify(this.optionsGet || {}, replacerSortKeys);
    }

    getQuery(): TGOptionsGet
    {
        return this.optionsGet as TGOptionsGet;
    }

    set(data: any, cb?: TGMessageCb, opt?: TGOptionsPut): TGLink
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

            soulSuffix = data.getPath()[0];
            value      = { '#': soulSuffix };
        }
        else if (isNode(data))
        {
            soulSuffix = data._['#'];
        }
        else
        {
            soulSuffix = generateMessageId();
        }

        this._addSoulSuffix(soulSuffix);
        return this.put(value, cb, opt);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _addSoulSuffix(path: string): void
    {
        path     = assertNotEmptyString(path);
        this.key = this.key.endsWith('/') ? `${this.key}${path}` : `${this.key}/${path}`;
        this._mergeSoul();
    }

    private _setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this.optionsGet['.'][key] = value;
    }

    private _mergeSoul(): void
    {
        this.optionsGet['#'] = this.getPath().shift();
    }
}
