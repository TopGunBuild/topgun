import type { PreparedStatement as SQLStatement } from '@sqlite.org/sqlite-wasm';
import { SQLLiteValue, Statement as IStatement } from '../types';

export class Statement implements IStatement
{
    constructor(private statement: SQLStatement)
    {
    }

    bind(values: any[])
    {
        return this.statement.bind(values);
    }

    finalize()
    {
        return this.statement.finalize();
    }

    get(values: SQLLiteValue[])
    {
        this.statement.bind(values as any);
        let step = this.statement.step();
        if (!step)
        { // no data available
            this.statement.reset();
            return undefined;
        }
        const results = this.statement.get({});
        this.statement.reset();
        return results;
    }

    run(values: SQLLiteValue[])
    {
        this.statement.bind(values as any);
        this.statement.stepReset();
    }

    reset()
    {
        return this.statement.reset();
    }

    all(values: SQLLiteValue[])
    {
        if (values && values.length > 0)
        {
            this.statement.bind(values as any);
        }


        let results = [];
        while (this.statement.step())
        {
            results.push(this.statement.get({}));
        }
        this.statement.reset();
        return results;
    }
}
