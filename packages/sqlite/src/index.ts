import { PublicKey } from '@topgunbuild/crypto';
import {
    CloseIteratorRequest, CollectNextRequest,
    SearchRequest,
    StoreInitProperties,
    StoreResult,
    StoreResults,
} from '@topgunbuild/store';
import { Database, SQLLite, Statement, Table } from './types';
import { getSQLTable } from './utils';
import {
    coerceSQLIndexType,
    coerceSQLType,
    convertSearchRequestToQuery,
    resolveFieldValues,
    resolveTable,
} from './schema';

export class SQLLiteStore
{
    db: Database;
    putStatement: Map<string, Statement>;
    primaryKeyArr: string[];
    cursor: Map<
        string,
        {
            kept: number;
            from: PublicKey;
            fetch: (
                amount: number,
            ) => Promise<{ results: StoreResult[]; kept: number }>;
            fetchStatement: Statement;
            countStatement: Statement;
            timeout: ReturnType<typeof setTimeout>;
        }
    >;
    iteratorTimeout: number;
    rootTableName = 'tg_node_field';
    closed        = true;
    properties: StoreInitProperties<any>;
    tables: Map<string, Table>;

    constructor(
        readonly sqllite: SQLLite,
        options?: {iteratorTimeout?: number}
    )
    {
        this.iteratorTimeout = options?.iteratorTimeout || 1e4;
    }

    async init(properties: StoreInitProperties<any>): Promise<void>
    {
        this.properties    = properties;
        this.primaryKeyArr = Array.isArray(properties.indexBy)
            ? properties.indexBy
            : [properties.indexBy];

        if (this.primaryKeyArr.length > 1)
        {
            throw new Error('Indexed by property can only be a root property');
        }

        if (!this.properties.schema)
        {
            throw new Error('Missing schema');
        }
    }

    async start(): Promise<void>
    {
        if (this.closed === false)
        {
            throw new Error('Already started');
        }
        this.closed = false;
        this.db     = await this.sqllite.createDatabase(undefined);

        const tables = getSQLTable(
            this.properties.schema!,
            [],
            this.primaryKeyArr[0]
        );

        this.rootTableName = tables[0].name;
        for (const table of tables)
        {
            const sql = `create table if not exists ${table.name}
                         (
                             ${[...table.fields, ...table.constraints].map((s) => s.definition).join(', ')}
                         )`
            this.db.exec(sql);
        }

        this.putStatement = new Map();
        this.tables       = new Map();
        for (const table of tables)
        {
            const sqlPut = `insert
            or replace into
            ${table.name}
            (
            ${table.fields.map((field) => field.name).join(', ')}
            )
            VALUES
            (
            ${table.fields.map((_x) => '?').join(', ')}
            );`;

            this.putStatement.set(table.name, await this.db.prepare(sqlPut));
            this.tables.set(table.name, table);
        }
        this.cursor = new Map();
    }

    async stop(): Promise<void>
    {
        if (this.closed)
        {
            return;
        }
        this.closed = true;
        for (const [_k, v] of this.putStatement)
        {
            v.finalize?.();
        }
        this.putStatement.clear();
        this.tables.clear();

        for (const [k, _v] of this.cursor)
        {
            this.clearupIterator(k);
        }
        await this.db.close();
    }

    async get(id: string): Promise<StoreResult|undefined>
    {
        const sql  = `select *
                      from ${this.rootTableName}
                      where ${this.primaryKeyArr[0]} = ? `;
        const stmt = await this.db.prepare(sql);
        const rows = await stmt.get([coerceSQLIndexType(id)]);
        stmt.finalize?.();
        return rows;
    }

    async put(value: StoreResult): Promise<void>
    {
        const valuesToPut = resolveFieldValues(
            value,
            resolveTable(this.tables, this.properties.schema)
        );

        for (const { table, values } of valuesToPut)
        {
            const statement = this.putStatement.get(table.name);
            if (!statement)
            {
                throw new Error('No statement found');
            }
            await statement.run(values.map((x: any) => typeof x === 'boolean' ? (x ? 1 : 0) : x));
        }
    }

    async del(id: string): Promise<void>
    {
        let statement = await this.db.prepare(`delete
                                               from ${this.rootTableName}
                                               where ${this.primaryKeyArr[0]} = ?`)
        await statement.run([coerceSQLType(id)])
        await statement.finalize?.();
    }

    async query(
        request: SearchRequest,
        from: PublicKey
    ): Promise<StoreResults>
    {
        // create a sql statement where the offset and the limit id dynamic and can be updated
        // TODO don't use offset but sort and limit 'next' calls by the last value of the sort
        const { where, join, orderBy } = convertSearchRequestToQuery(
            request,
            this.tables,
            this.tables.get(this.rootTableName)!
        );

        const query         = `${join ? join : ''} ${where ? where : ''}`;
        const sqlFetch      = `select ${this.rootTableName}.*
                               from ${this.rootTableName} ${query} ${orderBy ? orderBy : ''} limit ?
                               offset ?`;
        const stmt          = await this.db.prepare(sqlFetch);
        const totalCountKey = '__total_count';
        const sqlTotalCount = `select count(*) as ${totalCountKey}
                               from ${this.rootTableName} ${query}`;
        const countStmt     = await this.db.prepare(sqlTotalCount);

        let offset = 0;
        let first  = false;

        const fetch    = async (amount: number) =>
        {
            if (!first)
            {
                stmt.reset?.();
                countStmt.reset?.();

                // Bump timeout timer
                clearTimeout(iterator.timeout);
                iterator.timeout = setTimeout(
                    () => this.clearupIterator(request.idString),
                    this.iteratorTimeout
                );
            }

            first             = true;
            const offsetStart = offset;
            let results       = stmt.all([amount, offsetStart]);
            offset += amount;

            if (results.length > 0)
            {
                const totalCount = countStmt.get()[totalCountKey];
                iterator.kept    = totalCount - results.length - offsetStart;
            }
            else
            {
                iterator.kept = 0;
            }

            if (iterator.kept === 0)
            {
                this.clearupIterator(request.idString);
                clearTimeout(iterator.timeout);
            }
            return { results, kept: iterator.kept };
        };
        const iterator = {
            kept          : 0,
            fetch,
            from,
            fetchStatement: stmt,
            countStatement: countStmt,
            timeout       : setTimeout(
                () => this.clearupIterator(request.idString),
                this.iteratorTimeout
            )
        };

        this.cursor.set(request.idString, iterator);
        return fetch(request.fetch);
    }

    async next(
        query: CollectNextRequest,
        from: PublicKey
    ): Promise<StoreResults>
    {
        const cache = this.cursor.get(query.idString);
        if (!cache)
        {
            throw new Error('No statement found');
        }

        // reuse statement
        return cache.fetch(query.amount);
    }

    close(
        query: CloseIteratorRequest,
        from: PublicKey
    ): void|Promise<void>
    {
        this.clearupIterator(query.idString, from);
    }

    private clearupIterator(id: string, from?: PublicKey)
    {
        const cache = this.cursor.get(id);
        if (!cache)
        {
            return; // already cleared
        }
        if (from)
        {
            if (!cache.from.equals(from))
            {
                return; // wrong sender
            }
        }
        clearTimeout(cache.timeout);
        cache.countStatement.finalize?.();
        cache.fetchStatement.finalize?.();
        this.cursor.delete(id);
    }

    iterator(): IterableIterator<
        [string, StoreResult]
    >
    {
        throw new Error('Method not implemented.');
    }

    async getSize(): Promise<number>
    {
        const stmt   = await this.db.prepare(`select count(*) as total
                                              from ${this.rootTableName}`);
        const result = stmt.get()
        stmt.finalize?.();
        return result.total
    }

    getPending(cursorId: string): number|void
    {
        const cursor = this.cursor.get(cursorId);
        if (!cursor)
        {
            return;
        }
        return cursor.kept;
    }

    get cursorCount(): number
    {
        return this.cursor.size;
    }
}
