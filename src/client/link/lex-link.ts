import { TGLink } from './link';
import { TGData, TGOnCb, TGOptionsGet, TGValue } from '../../types';
import { TGStream } from '../../stream/stream';
import { TGLex } from './lex';

export class TGLexLink extends TGLex
{
    private readonly _link: TGLink;

    /**
     * Constructor
     */
    constructor(link: TGLink, maxLimit = 200, optionsGet: TGOptionsGet = {})
    {
        super(optionsGet, maxLimit);
        this._link      = link;
        this._link._lex = this;
        this.#setSoul();
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    start(value: string): TGLexLink
    {
        super.start(value);
        return this;
    }

    end(value: string): TGLexLink
    {
        super.end(value);
        return this;
    }

    prefix(value: string): TGLexLink
    {
        super.prefix(value);
        return this;
    }

    limit(value: number): TGLexLink
    {
        super.limit(value);
        return this;
    }

    reverse(value: boolean = true): TGLexLink
    {
        super.reverse(value);
        return this;
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

    #setSoul(): void
    {
        this.optionsGet['#'] = this._link.soul = this._link.getPath().join('/');
    }
}
