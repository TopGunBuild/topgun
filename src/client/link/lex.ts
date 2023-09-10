import { cloneValue, isNumber, isObject, isString } from '@topgunbuild/typed';
import { LEX, TGOptionsGet } from '../../types';
import { assertBoolean, assertNotEmptyString, assertNumber, replacerSortKeys } from '../../utils';

type KeyOfLex = keyof LEX;
type ValueOfLex = LEX[KeyOfLex];

export class TGLex
{
    readonly optionsGet: TGOptionsGet;
    readonly maxLimit: number;

    /**
     * Constructor
     */
    constructor(optionsGetOrSoul: TGOptionsGet|string, maxLimit = 200)
    {
        this.maxLimit  =  maxLimit;
        this.optionsGet = {
            '.': {},
            '%': this.maxLimit
        };
        if (isObject(optionsGetOrSoul))
        {
            if (isObject(optionsGetOrSoul['.']))
            {
                this.optionsGet['.'] = cloneValue(optionsGetOrSoul['.']);
            }
            if (isNumber(optionsGetOrSoul['%']))
            {
                this.optionsGet['%'] = optionsGetOrSoul['%'];
            }
        }
        else if (isString(optionsGetOrSoul))
        {
            this.optionsGet['#'] = optionsGetOrSoul;
        }
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    start(value: string): TGLex
    {
        this.#setLex('>', assertNotEmptyString(value));
        return this;
    }

    end(value: string): TGLex
    {
        this.#setLex('<', assertNotEmptyString(value));
        return this;
    }

    prefix(value: string): TGLex
    {
        this.#setLex('*', assertNotEmptyString(value));
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
        this.optionsGet['%'] = assertNumber(value);
        return this;
    }

    reverse(value = true): TGLex
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

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    #setLex(key: KeyOfLex, value: ValueOfLex): void
    {
        this.optionsGet['.'][key] = value;
    }
}

export function createLex(optionsGetOrSoul: TGOptionsGet|string, maxLimit = 200): TGLex
{
    return new TGLex(optionsGetOrSoul, maxLimit);
}