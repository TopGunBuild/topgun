import { SqlColumn } from './sql-column';
import { SqlIndex } from './sql-index';
import { SqlConstraint } from './sql-constraint.ts';

export class SqlTable
{
  name: string;
  columns: SqlColumn[];
  indexes: SqlIndex[];
  constraints: SqlConstraint[];

  static create(name: string): SqlTable
  {
    return new SqlTable(name);
  }

  get primaryColumnNames(): string
  {
    return this.columns
      .filter(col => col.primary)
      .map(col => col.name)
      .join();
  }

  constructor(tableName: string)
  {
    this.name    = `tg_${tableName}`;
    this.columns = [];
    this.indexes = [];
  }

  setIndexes(cb: (table: SqlTable) => SqlIndex[]): SqlTable
  {
    this.indexes = cb(this);
    return this;
  }

  setColumns(cb: (table: SqlTable) => SqlColumn[]): SqlTable
  {
    this.columns = cb(this);
    return this;
  }

  setConstraints(cb: (table: SqlTable) => SqlConstraint[]): SqlTable
  {
    this.constraints = cb(this);
    return this;
  }
}
