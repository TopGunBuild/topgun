export type SQLLite = {
    createDatabase: (directory?: string) => Promise<Database>|Database,
}

export type Database = {
    exec: (sql: string) => Promise<any>|any
    prepare: (sql: string, err?: (err: any) => any) => Promise<Statement>|Statement,
    close: (err?: (err: any) => any) => Promise<any>|any,
    get: (sql: string, err?: (err: any, row: any) => any) => Promise<any>|any,
    run: (sql: string, bind: any[], err?: (err: any) => any) => Promise<any>|any,
}

export type Statement = {
    bind: (values: SQLLiteValue[], err?: (err: any) => any) => Promise<any>|any
    finalize: (err?: (err: any) => any) => Promise<any>|any
    get: (values?: SQLLiteValue[], err?: (err: any, row: any) => any) => Promise<any>|any
    run: (values: SQLLiteValue[], err?: (err: any) => any) => Promise<any>|any
    reset: (err?: (err: any) => any) => Promise<any>|any,
    all: (values: SQLLiteValue[], err?: (err: any, rows: any[]) => any) => Promise<any>|any
}

export type SQLLiteValue =
    |string
    |number
    |null
    |Uint8Array
    |Int8Array
    |ArrayBuffer;

export type UpdateDeleteAction = 'cascade' | 'restrict' | 'no action' | 'set null' | 'set default';

export type SQLLiteColumn = {
    name: string,
    type?: string,
    primary?: boolean,
    target?: SQLLiteTable,
    targetColumn?: string;
    actions?: {
        onUpdate?: UpdateDeleteAction;
        onDelete?: UpdateDeleteAction;
    }
};

export type SQLConstraint = {
    definition: string
};

export class SQLLiteTable
{
    name: string;
    columns: SQLLiteColumn[];
    constraints: SQLConstraint[];

    static create(name: string): SQLLiteTable
    {
        return new SQLLiteTable(name);
    }

    get primaryColumnNames(): string
    {
        return this.columns
            .filter(col => col.primary)
            .map(col => col.name)
            .join();
    }

    constructor(name: string)
    {
        this.name    = name;
        this.columns = [];
        this.constraints = [];
    }

    setColumns(cb: (table: SQLLiteTable) => SQLLiteColumn[]): SQLLiteTable
    {
        this.columns = cb(this);
        return this;
    }

    setConstraints(cb: (table: SQLLiteTable) => SQLConstraint[]): SQLLiteTable
    {
        this.constraints = cb(this);
        return this;
    }
}
