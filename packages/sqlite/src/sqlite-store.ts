import { PublicKey } from '@topgunbuild/crypto';
import { IdKey, Store, StoreResults, StoreValue } from '@topgunbuild/store';
import { SelectQuery, SelectNextQuery, CloseIteratorQuery } from '@topgunbuild/transport';
import { Database, SQLLite, SQLLiteColumn, Statement } from './types';
import { convertSearchRequestToSQLQuery, resolveTableValues } from './schema';
import { extractIdKey } from './utils';

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
    rootTableColumns: SQLLiteColumn[]; // Record<keyof StoreValue, string>;

    constructor(
        readonly sqlLite: SQLLite,
        options?: { iteratorTimeout?: number, directory?: string },
    )
    {
        this.closed        = true;
        this.rootTableName = 'tg_entry';
        this.rootTableColumns = [
            { name: 'section', type: 'TEXT', primary: true },
            { name: 'node', type: 'TEXT', primary: true },
            { name: 'field', type: 'TEXT', primary: true },
            { name: 'state', type: 'REAL' },
            { name: 'value_is_empty', type: 'INTEGER' },
            { name: 'value_string', type: 'TEXT' },
            { name: 'value_bool', type: 'INTEGER' },
            { name: 'value_number', type: 'REAL' },
            { name: 'value_byte', type: 'BLOB' },
            { name: 'value_date', type: 'TEXT' },
            { name: 'deleted', type: 'INTEGER' },
        ];
        this.iteratorTimeout  = options?.iteratorTimeout || 1e4;
    }

    async start(): Promise<void>
    {
        if (this.closed === false)
        {
            throw new Error('Already started');
        }
        this.closed              = false;
        this.db                  = await this.sqlLite.createDatabase(this.directory);
        const columnNames        = this.rootTableColumns.map(c => c.name);
        const primaryColumnNames = this.rootTableColumns.filter(c => c.primary).map(c => c.name);

        const sql = `create table if not exists ${this.rootTableName}
        (
            ${columnNames.join(', ')},
            PRIMARY
            KEY
                     (
            ${primaryColumnNames.join(', ')}
                     )
            )

        create unique index if not exists tg_entry_section_node_field_uindex
            on tg_entry (section, node, field);

        create index if not exists tg_entry_section_node_index
            on tg_entry (section, node);

        create index if not exists tg_entry_section_index
            on tg_entry (section);

        create index if not exists tg_entry_deleted_index
            on tg_entry (deleted);`;
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
        const values    = resolveTableValues(value, this.rootTableColumns);
        await statement.run(values);
    }

    async get(id: IdKey): Promise<StoreValue[]>
    {
        const { values, columnNames } = extractIdKey(id);

        const sql  = `select *
                      from ${this.rootTableName}
                      where ${columnNames.map(x => `${x} = ?`).join(' and ')}`;
        const stmt = await this.db.prepare(sql);
        const rows = await stmt.get(values);
        stmt.finalize?.();
        return rows;
    }

    async del(id: IdKey): Promise<void>
    {
        const { values, columnNames } = extractIdKey(id);

        const sql       = `delete
                           from ${this.rootTableName}
                           where ${columnNames.map(x => `${x} = ?`).join(' and ')}`;
        const statement = await this.db.prepare(sql);
        await statement.run(values);
        await statement.finalize?.();
    }

    async select(message: SelectQuery, from: PublicKey): Promise<StoreResults>
    {
        const { where, join, sort } = convertSearchRequestToSQLQuery(
            message,
            this.rootTableName,
        );

        const query         = `${join ? join : ''} ${where ? where : ''}`;
        const sqlFetch      = `SELECT tge.*
                               FROM ${this.rootTableName} tge
                                        JOIN (SELECT section, node FROM ${this.rootTableName} ${query} limit ? offset ?) n
                                             ON tge.section = n.section and tge.node = n.node
                                   ${sort ? sort : ''}`;
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

    async next(query: SelectNextQuery, from: PublicKey): Promise<StoreResults>
    {
        const cache = this.cursor.get(query.id);
        if (!cache)
        {
            throw new Error('No statement found');
        }

        // reuse statement
        return cache.fetch(query.pageSize);
    }

    close(query: CloseIteratorQuery, from: PublicKey): void
    {
        this.clearUpIterator(query.id, from);
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
}
