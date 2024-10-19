import { DataChagesEvent } from "../live-data-grid/types";
import { DataUtil } from "./data-util";
import { DatasetState, RecordMetadata } from "./types";

/**
 * Class representing a dataset with CRUD operations and data processing capabilities.
 */
export class Dataset {
    public rawData: any[];
    private cbs: ((data: DataChagesEvent<any>) => void)[] = [];

    /**
     * Constructs a new Dataset instance.
     * @param initialData - The initial data to populate the dataset with.
     */
    constructor(initialData: any[] = []) {
        this.rawData = initialData;
    }

    /**
     * Adds a callback to the dataset.
     * @param cb - The callback to add.
     * @returns A function to remove the callback.
     */
    public onChanges(cb: (data: DataChagesEvent<any>) => void): () => void {
        this.cbs.push(cb);
        return () => {
            this.cbs = this.cbs.filter(c => c !== cb);
        };
    }

    /**
     * Applies transformations to the dataset.
     * @param config - The configuration for the transformations.
     * @returns The dataset with the applied transformations.
     */
    public process(config: DatasetState): {rows: any[], total: number} {
        return DataUtil.processDataset(this.rawData, config);
    }


    /**
     * Finds the index of a record in the dataset.
     * @param record - The record to find.
     * @param source - The source of the data to search in.
     * @returns The index of the record, or -1 if not found.
     */
    public findRecordIndex(record: object): number {
        return this.rawData.indexOf(record);
    }

    /**
     * Retrieves a record from the dataset by its index.
     * @param index - The index of the record to retrieve.
     * @param source - The source of the data to retrieve from.
     * @returns The record at the specified index, or undefined if not found.
     */
    public getRecordAt(index: number): object {
        return this.rawData[index];
    }

    /**
     * Finds a record in the dataset by a specific field value.
     * @param fieldName - The name of the field to search by.
     * @param value - The value to search for.
     * @param source - The source of the data to search in.
     * @returns The metadata of the record, or undefined if not found.
     */
    public findRecordByField(fieldName: string, value: any): RecordMetadata {
        const targetData = this.rawData;
        const dataLength = targetData.length;
        const result: RecordMetadata = {position: -1, data: undefined};
        
        for (let i = 0; i < dataLength; i++) {
            if (targetData[i][fieldName] === value) {
                result.position = i;
                result.data = targetData[i];
                break;
            }
        }
        return result;
    }

    /**
     * Inserts a record into the dataset.
     * @param record - The record to insert.
     * @param position - The position to insert the record at.
     */
    public insertRecord(record: object, position?: number): void {
        if (position === null || position === undefined) {
            this.rawData.push(record);
        } else {
            this.rawData.splice(position, 0, record);
        }
        this.emitChanges({operation: 'insert', rowData: record});
    }

    /**
     * Removes a record from the dataset.
     * @param record - The record to remove.
     * @returns True if the record was removed, false otherwise.
     */
    public removeRecord(record: object): boolean {
        const index: number = this.findRecordIndex(record);
        return this.removeRecordAt(index);
    }

    /**
     * Removes a record from the dataset by its index.
     * @param index - The index of the record to remove.
     * @returns True if the record was removed, false otherwise.
     */
    public removeRecordAt(index: number): boolean {
        const result = this.rawData.splice(index, 1).length === 1;
        if (result) {
            this.emitChanges({operation: 'delete', rowData: this.rawData[index]});
        }
        return result;
    }

    /**
     * Modifies a record in the dataset by its index.
     * @param index - The index of the record to modify.
     * @param updatedProperties - The properties to update.
     * @returns The modified record, or undefined if not found.
     */
    public modifyRecordAt(index: number, updatedProperties: object): object {
        const existingRecord = this.getRecordAt(index);
        if (!existingRecord) {
            return undefined;
        }
        const result = Object.assign(existingRecord, updatedProperties);
        this.emitChanges({operation: 'update', rowData: result, oldData: existingRecord});
        return result;
    }

    /**
     * Emits changes to the dataset.
     * @param data - The data to emit.
     */
    private emitChanges(data: DataChagesEvent<any>): void {
        this.cbs.forEach(cb => cb(data));
    }
}
