import { Link } from './link';
import { LEX } from '../types/lex';
import { isObject } from '../utils/is-object';
import { isString } from '../utils/is-string';
import { Client } from './client';
import { isNumber } from '../utils/is-number';
import { OptionsGet, Value } from '../types';
import { cloneValue } from '../utils/clone-value';

type KeyOfLex = keyof LEX;
type ValueOfLex = LEX[KeyOfLex];

export class LexLink extends Link
{
    /**
     * Constructor
     */
    constructor(chain: Client, key: string)
    {
        super(chain, key);
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
    get(query: OptionsGet): LexLink
    get(key: string): Link
    get(keyOrOptions: string|OptionsGet): Link|LexLink
    {
        // The argument is a LEX query
        if (isObject(keyOrOptions) && !isString(keyOrOptions))
        {
            const soul      = this.getPath().shift();
            this.optionsGet = { ['#']: soul, ['.']: {} };

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
            return new Link(this._chain, keyOrOptions as string, this);
        }
    }

    start(value: string): LexLink
    {
        this._setLex('>', value);
        return this;
    }

    end(value: string): LexLink
    {
        this._setLex('<', value);
        return this;
    }

    prefix(value: string): LexLink
    {
        this._setLex('*', value);
        return this;
    }

    equals(value: string): LexLink
    {
        this._setLex('=', value);
        return this;
    }

    limit(value: number): LexLink
    {
        this.optionsGet['%'] = value;
        return this;
    }

    reverse(value = true): LexLink
    {
        this.optionsGet['-'] = value;
        return this;
    }

    map(): Link
    {
        return super.map();
    }

    toString(): string
    {
        return JSON.stringify(this.optionsGet);
    }

    getQuery(): OptionsGet
    {
        return this.optionsGet;
    }

    on(cb: (node: (Value|undefined), key?: string) => void): Link
    {
        return super.on(cb);
    }

    once(cb: (node: (Value|undefined), key?: string) => void): Link
    {
        return super.once(cb);
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    private _setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this.optionsGet['.'][key] = value;
    }
}
