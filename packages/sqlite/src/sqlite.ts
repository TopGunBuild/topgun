import * as DB from 'better-sqlite3';
import type { Database as IDatabase } from './types';

export const createDatabase = async (directory?: string) =>
{
    const db = new DB(directory || ':memory:');

    db.pragma('journal_mode = WAL');
    return db as any as IDatabase;
}

