import { PublicKey } from '@topgunbuild/crypto';
import { IdKey, Store, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectNextQuery, CloseIteratorQuery } from '@topgunbuild/transport';
import { Database, SQLLite, Statement } from './types';
import { convertSearchRequestToSQLQuery, resolveTableValues } from './schema';

export class SQLLiteStore implements Store
{
    db: Database;
    putStatement: Map<string, Statement>;
    cursor: Map<
        string,
        {
            left: number;
            from: PublicKey;
            fetch: (
                amount: number,
            ) => Promise<{ results: StoreValue[]; left: number }>;
            fetchStatement: Statement;
            countStatement: Statement;
            timeout: ReturnType<typeof setTimeout>;
        }
    >;
    iteratorTimeout: number;
    directory: string;
    closed: boolean;
    rootTableName: string;
    rootTableFields: Record<string, string>;

    constructor(
        readonly sqlLite: SQLLite,
        options?: { iteratorTimeout?: number, directory?: string },
    )
    {
        this.closed          = true;
        this.rootTableName   = 'tg_node_field';
        this.rootTableFields = {
            node_name : 'text',
            field_name: 'text',
            value     : 'any',
            state     : 'text',
            size      : 'integer',
            type      : 'integer',
            deleted   : 'integer',
        };
        this.iteratorTimeout = options?.iteratorTimeout || 1e4;
    }

    async start(): Promise<void>
    {
        if (this.closed === false)
        {
            throw new Error('Already started');
        }
        this.closed = false;
        this.db     = await this.sqlLite.createDatabase(this.directory);

        const columnNames = Object.keys(this.rootTableFields);
        const sql         = `create table if not exists ${this.rootTableName}
                             (
                                 ${columnNames.join(', ')}
                             )

        create unique index if not exists ${this.rootTableFields}_node_name_field_name_uindex
            on ${this.rootTableFields} (node_name, field_name);

        create index if not exists ${this.rootTableFields}_node_name_index
            on ${this.rootTableFields} (node_name);

        create index if not exists ${this.rootTableFields}_deleted_index
            on ${this.rootTableFields} (deleted);`;
        this.db.exec(sql);

        this.putStatement = new Map();

        const sqlPut = `insert
        or replace into
        ${this.rootTableName}
        (
        ${columnNames.join(', ')}
        )
        VALUES
        (
        ${columnNames.map((_x) => '?').join(', ')}
        );`;

        this.putStatement.set(this.rootTableName, await this.db.prepare(sqlPut));

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

        for (const [k, _v] of this.cursor)
        {
            this.clearUpIterator(k);
        }
        await this.db.close();
    }

    async put(value: StoreValue): Promise<void>
    {
        const statement = this.putStatement.get(this.rootTableName);
        const values    = resolveTableValues(value, this.rootTableFields);
        await statement.run(values);
    }

    async get(id: IdKey): Promise<StoreValue|undefined>
    {
        const sql  = `select *
                      from ${this.rootTableName}
                      where node_name = ? and field_name = ?`;
        const stmt = await this.db.prepare(sql);
        const rows = await stmt.get([id.node_name, id.field_name]);
        stmt.finalize?.();
        return rows;
    }

    async del(id: IdKey): Promise<void>
    {
        let statement = await this.db.prepare(`delete
                                               from ${this.rootTableName}
                                               where node_name = ? and field_name = ?`);
        await statement.run([id.node_name, id.field_name]);
        await statement.finalize?.();
    }

    async select(message: SelectQuery, from: PublicKey): Promise<StoreResults>
    {
        const { where, join, orderBy } = convertSearchRequestToSQLQuery(
            message,
            this.rootTableName
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
                    () => this.clearUpIterator(message.id),
                    this.iteratorTimeout,
                );
            }

            first             = true;
            const offsetStart = offset;
            let results       = stmt.all([amount, offsetStart]);
            offset += amount;

            if (results.length > 0)
            {
                const totalCount = countStmt.get()[totalCountKey];
                iterator.left    = totalCount - results.length - offsetStart;
            }
            else
            {
                iterator.left = 0;
            }

            if (iterator.left === 0)
            {
                this.clearUpIterator(message.id);
                clearTimeout(iterator.timeout);
            }
            return { results, left: iterator.left };
        };
        const iterator = {
            left          : 0,
            fetch,
            from,
            fetchStatement: stmt,
            countStatement: countStmt,
            timeout       : setTimeout(
                () => this.clearUpIterator(message.id),
                this.iteratorTimeout,
            ),
        };

        this.cursor.set(message.id, iterator);
        return fetch(message.pageSize);
    }

    async next(
        query: SelectNextQuery,
        from: PublicKey,
    ): Promise<StoreResults>
    {
        const cache = this.cursor.get(query.id);
        if (!cache)
        {
            throw new Error('No statement found');
        }

        // reuse statement
        return cache.fetch(query.pageSize);
    }

    close(query: CloseIteratorQuery, from: PublicKey): void|Promise<void>
    {
        this.clearUpIterator(query.id, from);
    }

    private clearUpIterator(id: string, from?: PublicKey)
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
        [string, StoreValue]
    >
    {
        throw new Error('Method not implemented.');
    }

    async getSize(): Promise<number>
    {
        const stmt   = await this.db.prepare(`select count(*) as total
                                              from ${this.rootTableName}`);
        const result = stmt.get();
        stmt.finalize?.();
        return result.total;
    }

    getPending(cursorId: string): number|undefined
    {
        const cursor = this.cursor.get(cursorId);
        if (!cursor)
        {
            return undefined;
        }
        return cursor.left;
    }

    get cursorCount(): number
    {
        return this.cursor.size;
    }
}
