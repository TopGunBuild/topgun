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
                '%': this.maxLimit,
                ...optionsGetOrSoul,
            };
        }
        else if (isString(optionsGetOrSoul))
        {
            this.options['#'] = optionsGetOrSoul;
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    start(value: string): TGLex
    {
        this.options['>'] = assertNotEmptyString(value);
        return this;
    }

    end(value: string): TGLex
    {
        this.options['<'] = assertNotEmptyString(value);
        return this;
    }

    prefix(value: string): TGLex
    {
        this.options['*'] = assertNotEmptyString(value);
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
        this.options['%'] = assertNumber(value);
        return this;
    }

    reverse(value = true): TGLex
    {
        this.options['-'] = assertBoolean(value);
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
}
