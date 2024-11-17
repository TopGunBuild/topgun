export class MiddlewareSystem<TInput, TOutput = TInput>
{
    readonly name: string;
    private readonly _middlewareFunctions: Array<(a: TInput) => Promise<TOutput>|TOutput|undefined>;

    /**
     * Constructor
     */
    constructor(name = 'MiddlewareSystem')
    {
        this.name                 = name;
        this._middlewareFunctions = [];
    }

    /**
     * Register middleware function
     *
     * @param middleware The middleware function to add
     */
    use(middleware: (a: TInput) => Promise<TOutput>|TOutput|undefined): MiddlewareSystem<TInput, TOutput>
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
        middleware: (a: TInput) => TOutput|undefined,
    ): MiddlewareSystem<TInput, TOutput>
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
    async process(a: TInput): Promise<TOutput|undefined|void>
    {
        let val: TInput|TOutput|undefined = a;

        for (const fn of this._middlewareFunctions)
        {
            if (!val)
            {
                return;
            }

            val = await fn(val as TInput);
        }

        return val as TOutput|undefined;
    }
}
