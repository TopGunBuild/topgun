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
