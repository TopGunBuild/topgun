import { SqlColumn } from './sql-column';
import { SqlIndex } from './sql-index';
import { SqlConstraint } from './sql-constraint';

/**
 * Represents a SQL table
 */
export class SqlTable
{
  name: string;
  columns: SqlColumn[];
  indexes: SqlIndex[];
  constraints: SqlConstraint[];

  /**
   * Create a new table
   * @param name - The name of the table
   * @returns A new SqlTable instance
   */
  static create(name: string): SqlTable
  {
    return new SqlTable(name);
  }

  /**
   * Get the names of the primary columns
   * @returns A string of primary column names
   */
  get primaryColumnNames(): string
  {
    return this.columns
      .filter(col => col.primary)
      .map(col => col.name)
      .join();
  }

  /**
   * Constructor for the SqlTable class
   * @param tableName - The name of the table
   */
  constructor(tableName: string)
  {
    this.name    = `tg_${tableName}`;
    this.columns = [];
    this.indexes = [];
  }

  /**
   * Set the indexes for the table
   * @param cb - A callback function that returns an array of SqlIndex instances
   * @returns The SqlTable instance
   */
  setIndexes(cb: (table: SqlTable) => SqlIndex[]): SqlTable
  {
    this.indexes = cb(this);
    return this;
  }

  /**
   * Set the columns for the table
   * @param cb - A callback function that returns an array of SqlColumn instances
   * @returns The SqlTable instance
   */
  setColumns(cb: (table: SqlTable) => SqlColumn[]): SqlTable
  {
    this.columns = cb(this);
    return this;
  }

  /**
   * Set the constraints for the table
   * @param cb - A callback function that returns an array of SqlConstraint instances
   * @returns The SqlTable instance
   */
  setConstraints(cb: (table: SqlTable) => SqlConstraint[]): SqlTable
  {
    this.constraints = cb(this);
    return this;
  }
}
