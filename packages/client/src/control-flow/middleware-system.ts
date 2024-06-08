export class MiddlewareSystem<T>
{
    readonly name: string;
    private readonly _middlewareFunctions: Array<(a: T) => Promise<T>|T|undefined>;

    /**
     * Constructor
     */
    constructor(name = 'MiddlewareSystem')
    {
        this.name                 = name;
        this._middlewareFunctions = [];
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    /**
     * Register middleware function
     *
     * @param middleware The middleware function to add
     */
    use(middleware: (a: T) => Promise<T>|T|undefined,): MiddlewareSystem<T>
    {
        if (this._middlewareFunctions.indexOf(middleware) !== -1)
        {
            return this;
        }

        this._middlewareFunctions.push(middleware);
        return this;
    }

    /**
     * Unregister middleware function
     *
     * @param middleware The middleware function to remove
     */
    unuse(
        middleware: (a: T) => T|undefined,
    ): MiddlewareSystem<T>
    {
        const idx = this._middlewareFunctions.indexOf(middleware);
        if (idx !== -1)
        {
            this._middlewareFunctions.splice(idx, 1);
        }

        return this;
    }

    /**
     * Process values through this middleware
     * @param a Required, this is the value modified/passed through each middleware fn
     */
    async process(a: T): Promise<T|undefined|void>
    {
        let val: T|undefined = a;

        for (const fn of this._middlewareFunctions)
        {
            if (!val)
            {
                return;
            }

            val = await fn(val);
        }

        return val;
    }
}
