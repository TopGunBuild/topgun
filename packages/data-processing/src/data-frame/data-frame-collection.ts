import { isDefined } from '@topgunbuild/utils';
import { RowComparator } from './types';
import { SortingCriteria } from '../sorting/types';
import { DataSortingEngine } from '../sorting/engine';

/**
 * Class contains a set of in-memory data along with methods for efficiently manipulating the data.
 * @class DataFrameCollection
 * @template T
 */
export class DataFrameCollection<T> {
    private data: T[];
    private readonly sortingCriteria: SortingCriteria[];
    private readonly sortingEngine: DataSortingEngine;
    private readonly compareRowsCb: RowComparator<T>;

    readonly pageSize: number;

    // Property to indicate whether the last request returned data
    hasLastRequestData: boolean = false;

    /**
     * Constructor for DataFrameCollection
     * @param {object} params - The parameters for initializing the DataFrameCollection
     * @param {SortingExpression[]} params.sortingExpressions - The sorting expressions for the data
     * @param {RowComparator<T>} params.compareRowsFn - The function to compare rows
     * @param {number} [params.pageSize] - The page size for pagination (optional)
     */
    constructor(params: {
        sortingCriteria: SortingCriteria[],
        compareRowsCb: RowComparator<T>,
        pageSize?: number
    }) {
        this.data = [];
        this.sortingEngine = new DataSortingEngine();
        this.pageSize = params.pageSize;
        this.compareRowsCb = params.compareRowsCb;
        this.sortingCriteria = params.sortingCriteria.map(s => {
            s.caseSensitive = true;
            return s;
        });
    }

    /**
     * Method to get the size of the data collection
     * @returns {number}
     */
    getDataSize(): number {
        return this.data.length;
    }

    /**
     * Method to get a copy of the data collection
     * @returns {T[]}
     */
    getData(): T[] {
        return [...this.data]; // Return a shallow copy of the data array
    }

    /**
     * Method to initialize the data collection with an array of data
     * @param {T[]} data
     */
    init(data: T[]): void {
        this.data = data;
        this.hasLastRequestData = this.data.length > 0; // Update the flag based on data length
    }

    /**
     * Method to remove items from the data collection starting at a specific index
     * @param {number} start
     * @param {number} deleteCount
     * @returns {T[]}
     */
    splice(start: number, deleteCount: number): T[] {
        return this.data.splice(start, deleteCount); // Remove and return the specified items
    }

    /**
     * Method to add an item to the start of the data collection
     * @param {T} item
     */
    setToStart(item: T): void {
        if (item) {
            this.data.unshift(item);
        }
    }

    /**
     * Method to add an item to the end of the data collection
     * @param {T} item
     */
    setToEnd(item: T): void {
        if (item) {
            this.data.push(item);
        }
    }

    /**
     * Method to insert a row into the data collection and sort it
     * @param {T} row
     */
    insert(row: T): void {
        if (row) {
            this.data = this.sort([row, ...this.data]);
        }
    }

    /**
     * Method to update an existing row in the data collection
     * @param {T} row
     */
    update(row: T): void {
        const index = this.data.findIndex(_row => this.compareRowsCb(_row, row));
        if (index > -1) {
            this.data[index] = row; // Update the row if found
        }
    }

    /**
     * Method to delete a row from the data collection
     * @param {T} row
     */
    delete(row: T): void {
        const index = this.data.findIndex(_row => this.compareRowsCb(_row, row));
        if (index > -1) {
            this.data.splice(index, 1); // Remove the row if found
        }
    }

    /**
     * Method to remove and return the first item in the data collection
     * @returns {T | undefined}
     */
    firstRemove(): T | undefined {
        return this.data.shift();
    }

    /**
     * Method to remove and return the last item in the data collection
     * @returns {T | undefined}
     */
    lastRemove(): T | undefined {
        return this.data.pop();
    }

    /**
     * Method to get the first item in the data collection
     * @returns {T | undefined}
     */
    getFirst(): T | undefined {
        return this.data[0];
    }

    /**
     * Method to get the last item in the data collection
     * @returns {T | undefined}
     */
    getLast(): T | undefined {
        return this.data[this.data.length - 1];
    }

    /**
     * Method to get the first and last items in the data collection
     * @returns {T[]}
     */
    getShort(): T[] {
        if (isDefined(this.getFirst()) && isDefined(this.getLast())) {
            return [this.getFirst()!, this.getLast()!]; // Return an array with the first and last items
        }
        else {
            return []; // Return an empty array if either is undefined
        }
    }

    /**
     * Method to check if a row is before the first item in the data collection
     * @param {T} row
     * @returns {boolean}
     */
    isBefore(row: T): boolean {
        if (this.getDataSize() === 0) {
            return false; // Return false if the collection is empty
        }
        const array = this.sort([...this.getShort(), row]);
        return this.compareRowsCb(array[0], row) && !this.compareRowsCb(array[1], row); // Check if the row is the first and not equal to the second
    }

    /**
     * Method to check if a row belongs to the data collection
     * @param {T} row
     * @returns {boolean}
     */
    isBelong(row: T): boolean {
        if (this.getDataSize() === 0) {
            return false; // Return false if the collection is empty
        }
        if (this.compareRowsCb(row, this.getFirst())) {
            return true; // Return true if the row is the first item
        }
        if (this.compareRowsCb(row, this.getLast())) {
            return true; // Return true if the row is the last item
        }

        const array = this.sort([...this.getShort(), row]);
        return this.compareRowsCb(array[1], row); // Check if the row is the second item
    }

    /**
     * @param {T[]} rows
     * @returns {T[]}
     * @private
     */
    private sort(rows: T[]): T[] {
        return this.sortingEngine.process(rows, this.sortingCriteria);
    }
}
