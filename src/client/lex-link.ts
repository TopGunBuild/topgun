import { cloneValue, isNumber, isString, isObject, isNotEmptyObject } from 'topgun-typed';
import { TGLink } from './link';
import { LEX } from '../types/lex';
import { TGClient } from './client';
import { TGMessageCb, TGOptionsGet, TGOptionsPut, TGValue } from '../types';
import { assertBoolean, assertNotEmptyString, assertNumber } from '../utils/assert';
import { generateMessageId } from './graph/graph-utils';

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

    map(): TGLink
    {
        return super.map();
    }

    toString(): string
    {
        return JSON.stringify(this.optionsGet || {});
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

    set(data: any, cb?: TGMessageCb, opt?: TGOptionsPut): TGLink
    {
        let soul;
        const put = (soul: string, value: TGValue) =>
        {
            if (this.userPubExpected())
            {
                throw new Error(
                    'You cannot save data to user space if the user is not authorized.',
                );
            }

            this._chain.graph.putPath(
                [...this.getPath(), soul],
                value,
                cb,
                opt,
            );
        };

        if (data instanceof TGLink && data.optionsGet['#'])
        {
            soul = data.optionsGet['#'];
            put(soul, {
                '#': soul,
            }
            );
        }
        else if (data && data._ && data._['#'])
        {
            soul = data && data._ && data._['#'];
            put(soul, data);
        }
        else if (isObject(data) && isNotEmptyObject(data))
        {
            soul = generateMessageId();
            put(soul, data);
        }
        else
        {
            throw new Error('This data type is not supported in set()');
        }

        return this;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this._persistOptions();
        this.optionsGet['.'][key] = value;
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
