import sqlitePromise, { Database as SQLDatabase, PreparedStatement as SQLStatement } from '@sqlite.org/sqlite-wasm';
import type { Database as IDatabase } from '../types';
import { Statement } from './statement';

class Database implements IDatabase
{
    statements: Map<string, SQLStatement> = new Map();

    constructor(private db: SQLDatabase)
    {
    }

    async exec(sql: string)
    {
        return this.db.exec(sql);
    }

    async prepare(sql: string): Promise<Statement>
    {
        const statement = this.db.prepare(sql);
        this.statements.set(sql, statement);
        return new Statement(statement);
    }

    async close()
    {
        return this.db.close();
    }

    async get(sql: string)
    {
        return this.db.exec({ sql, rowMode: 'array' });
    }

    async run(sql: string, bind: any[])
    {
        return this.db.exec(sql, { bind, rowMode: 'array' });
    }
}

export const createDatabase = async (directory?: string) =>
{
    const log    = (...args: any) => console.log(...args);
    const error  = (...args: any) => console.error(...args);
    const sqlite = await sqlitePromise({ print: log, printErr: error });

    return new Database(new sqlite.oo1.DB(directory ?? ':memory:', 'c'));
};
