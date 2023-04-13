export class TGMiddlewareSystem<T, U = undefined, V = undefined> 
{
    readonly name: string;
    private readonly _middlewareFunctions: Array<
        (a: T, b?: U, c?: V) => Promise<T> | T | undefined
    >;

    /**
     * Constructor
     */
    constructor(name = 'MiddlewareSystem') 
    {
        this.name = name;
        this._middlewareFunctions = [];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    use(
        middleware: (a: T, b?: U, c?: V) => Promise<T> | T | undefined,
    ): TGMiddlewareSystem<T, U, V> 
    {
        if (this._middlewareFunctions.indexOf(middleware) !== -1) 
        {
            return this;
        }

        this._middlewareFunctions.push(middleware);
        return this;
    }

    unuse(
        middleware: (a: T, b?: U, c?: V) => T | undefined,
    ): TGMiddlewareSystem<T, U, V> 
    {
        const idx = this._middlewareFunctions.indexOf(middleware);
        if (idx !== -1) 
        {
            this._middlewareFunctions.splice(idx, 1);
        }

        return this;
    }

    async process(a: T, b?: U, c?: V): Promise<T | undefined | void> 
    {
        let val: T | undefined = a;

        for (const fn of this._middlewareFunctions) 
        {
            if (!val) 
            {
                return;
            }

            val = await fn(val, b, c);
        }

        return val;
    }
}
