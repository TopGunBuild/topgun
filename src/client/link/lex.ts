import { isObject, isString } from '@topgunbuild/typed';
import { TGOptionsGet } from '../../types';
import { assertBoolean, assertNotEmptyString, assertNumber, replacerSortKeys } from '../../utils';

export class TGLex
{
    readonly options: TGOptionsGet;
    readonly maxLimit: number;

    /**
     * Constructor
     */
    constructor(optionsGetOrSoul: TGOptionsGet|string, maxLimit = 200)
    {
        this.maxLimit = maxLimit;
        this.options  = {};
        if (isObject(optionsGetOrSoul))
        {
            this.options = {
                limit: this.maxLimit,
                ...optionsGetOrSoul,
            };
        }
        else if (isString(optionsGetOrSoul))
        {
            this.options.equals = optionsGetOrSoul;
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    start(value: string): TGLex
    {
        this.#setLex('start', assertNotEmptyString(value));
        return this;
    }

    end(value: string): TGLex
    {
        this.#setLex('end', assertNotEmptyString(value));
        return this;
    }

    prefix(value: string): TGLex
    {
        this.#setLex('prefix', assertNotEmptyString(value));
        return this;
    }

    limit(value: number): TGLex
    {
        if (value > this.maxLimit)
        {
            throw Error(
                `Limit exceeds the maximum allowed. The maximum length is ${this.maxLimit}`
            );
        }
        this.#setLex('limit', assertNumber(value));
        return this;
    }

    reverse(value = true): TGLex
    {
        this.#setLex('reverse', assertBoolean(value));
        return this;
    }

    toString(): string
    {
        return JSON.stringify(this.options, replacerSortKeys);
    }

    getQuery(): TGOptionsGet
    {
        return this.options;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #setLex(key: keyof TGOptionsGet, value: TGOptionsGet[keyof TGOptionsGet]): void
    {
        this.options['.'][key] = value;
    }
}
