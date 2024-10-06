import { AsyncQueue } from '@topgunbuild/utils';
import {
    DatabaseOutputData,
    DatabaseQueryFunction,
    DatabaseChangesToRowConverter,
    StreamChangesFunction,
    StreamProcessingParams, UniqueIdentifierExtractor,
} from './types';
import { StreamDataCollection } from './collection.ts';
import { SelectMessagesAction } from '@topgunbuild/types';
import { FilterExpressionTree } from './filtering';
import { convertSelectToFilterExpressionTree } from './utils/convert-select.ts';

export class StreamProcessing<T, D = null>
{
    readonly query: SelectMessagesAction;
    readonly queryFunction: DatabaseQueryFunction<T>;
    readonly emitChanges: StreamChangesFunction<T>;
    readonly queue: AsyncQueue;
    readonly filterExpressionTree: FilterExpressionTree;
    readonly dataToRowConverter: DatabaseChangesToRowConverter<D, T>;
    readonly identifierExtractor: UniqueIdentifierExtractor<T>;

    readonly rowsBefore: StreamDataCollection<T>;
    readonly rowsAfter: StreamDataCollection<T>;
    readonly rowsCurrent: StreamDataCollection<T>;

    rowAdded: T;
    rowDeleted: T;

    constructor(params: StreamProcessingParams<T, D>)
    {
        const {
                  query,
                  query: { sort: sortingExpressions, pageOffset },
                  compareRows,
                  additionalRowsBefore,
                  additionalRowsAfter,
                  queryFunction,
                  emitChanges,
                  databaseChangesToRowConverter,
                  identifierExtractor,
              } = params;

        this.query                = query;
        this.queryFunction        = queryFunction;
        this.emitChanges          = emitChanges;
        this.dataToRowConverter   = databaseChangesToRowConverter;
        this.identifierExtractor  = identifierExtractor;
        this.filterExpressionTree = convertSelectToFilterExpressionTree(query);
        this.queue                = new AsyncQueue();

        this.rowsBefore  = new StreamDataCollection({
            sortingExpressions,
            compareRows,
            additionalRows: additionalRowsBefore > pageOffset ? pageOffset : additionalRowsBefore,
            identifierExtractor,
        });
        this.rowsAfter   = new StreamDataCollection({
            sortingExpressions,
            compareRows,
            additionalRows: additionalRowsAfter,
            identifierExtractor,
        });
        this.rowsCurrent = new StreamDataCollection({
            sortingExpressions,
            compareRows,
            identifierExtractor,
        });
    }

    async fetchFromDatabase(emitChanges = false)
    {
        const query      = this.query;
        query.pageOffset = this.query.pageOffset - this.rowsBefore.additionalRows;
        query.pageSize   = this.query.pageSize + this.rowsBefore.additionalRows + this.rowsAfter.additionalRows;

        const queryResult = await this.queryFunction(query);

        this.rowsCurrent.init(queryResult.rows);

        this.rowsBefore.init(
            this.rowsCurrent.splice(0, this.rowsBefore.additionalRows),
        );
        this.rowsAfter.init(
            this.rowsCurrent.splice(this.query.pageSize, this.rowsAfter.additionalRows),
        );

        if (emitChanges)
        {
            this.#emitChanges(true);
        }
    }

    databaseOutput(value: DatabaseOutputData<D>)
    {
        const { operation, rowData, oldData } = value;
        const row                             = this.dataToRowConverter(rowData);
        const oldRow                          = this.dataToRowConverter(oldData);

        switch (operation)
        {
            case 'insert':
                break;

            case 'update':
                break;

            case 'delete':
                break;
        }
    }

    async updateHandler(row: T, oldRow: T): Promise<void>
    {
        await this.deleteHandler(oldRow, false);
        await this.insertHandler(row, false);

        this.rowAdded   = row;
        this.rowDeleted = oldRow;
        this.#emitChanges();
    }

    async insertHandler(row, emitChanges = true)
    {
        this.#clearPreviousValues();

        if (this.#needSyncWithDB())
        {
            await this.fetchFromDatabase(emitChanges);
        }
        else
        {
            const hasBefore = this.query.pageOffset > 0;

            switch (true)
            {
                case hasBefore && this.rowsBefore.isBefore(row):
                    await this.#shiftUp();
                    break;

                case hasBefore && this.rowsBefore.isBelong(row):
                    this.rowsBefore.insert(row);
                    await this.#shiftUp();
                    break;

                case this.rowsCurrent.isBefore(row):
                    this.rowsCurrent.setToStart(row);
                    this.rowAdded = row;
                    await this.#shiftFromCurrentToAfter();
                    break;

                case this.rowsCurrent.isBelong(row):
                    this.rowsCurrent.insert(row);
                    this.rowAdded = row;
                    await this.#shiftFromCurrentToAfter();
                    break;

                case this.rowsAfter.isBefore(row):
                    this.rowsAfter.setToStart(row);
                    break;

                case this.rowsAfter.isBelong(row):
                    this.rowsAfter.insert(row);
                    break;
            }

            if (emitChanges)
            {
                this.#emitChanges();
            }
        }
    }

    async deleteHandler(row, emitChanges = true)
    {
        this.#clearPreviousValues();

        if (this.#needSyncWithDB())
        {
            await this.fetchFromDatabase(emitChanges);
        }
        else
        {
            switch (true)
            {
                case this.rowsBefore.isBefore(row):
                    await this.#shiftDown();
                    break;

                case this.rowsBefore.isBelong(row):
                    await this.#shiftDown();
                    this.rowsBefore.delete(row);
                    break;

                case this.rowsCurrent.isBelong(row):
                    await this.#deleteFromCurrent(row);
                    break;

                case this.rowsAfter.isBelong(row):
                    this.rowsAfter.delete(row);
                    break;
            }

            if (emitChanges)
            {
                this.#emitChanges();
            }
        }
    }

    #emitChanges(persist = false): void
    {
        if (persist || this.rowAdded || this.rowDeleted)
        {
            this.emitChanges({
                added     : this.rowAdded,
                deleted   : this.rowDeleted,
                collection: this.rowsCurrent.getData(),
            });
        }
    }

    #needSyncWithDB(): boolean
    {
        return (this.rowsBefore.getDataSize() === 0 && this.rowsBefore.hasLastRequestData) //  && this.selectParams.offset > 0
            || this.rowsAfter.getDataSize() === 0 && this.rowsAfter.hasLastRequestData
            || this.rowsCurrent.getDataSize() === 0 && this.rowsCurrent.hasLastRequestData;
    }

    #clearPreviousValues(): void
    {
        this.rowAdded   = null;
        this.rowDeleted = null;
    }

    async #deleteFromCurrent(row: T): Promise<void>
    {
        this.rowsCurrent.delete(row);
        this.rowDeleted = row;
        await this.#shiftFromAfterToCurrent();
    }

    async #shiftUp(): Promise<void>
    {
        await this.#shiftFromBeforeToCurrent();
        await this.#shiftFromCurrentToAfter();
    }

    #shiftFromBeforeToCurrent()
    {
        this.rowAdded = this.rowsBefore.lastRemove();
        this.rowsCurrent.setToStart(this.rowAdded);
    }

    #shiftFromCurrentToAfter(): void
    {
        if (this.rowsCurrent.getDataSize() > this.query.pageSize)
        {
            this.rowDeleted = this.rowsCurrent.lastRemove();
            this.rowsAfter.setToStart(this.rowDeleted);
        }
    }

    async #shiftDown(): Promise<void>
    {
        await this.#shiftFromAfterToCurrent();
        await this.#shiftFromCurrentToBefore();
    }

    #shiftFromAfterToCurrent(): void
    {
        this.rowAdded = this.rowsAfter.firstRemove();
        this.rowsCurrent.setToEnd(this.rowAdded);
    }

    #shiftFromCurrentToBefore(): void
    {
        this.rowDeleted = this.rowsCurrent.firstRemove();
        this.rowsBefore.setToEnd(this.rowDeleted);
    }
}
