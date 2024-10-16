import { DataGenerator } from "@topgunbuild/test-utils";
import { Dataset, DataSource } from "../dataset";
import { FilteringOperator, FilteringState, NUMBER_FILTER_CONDITIONS, NumberCondition } from "../filtering";
import { SortDirection } from "@topgunbuild/types";
import { SortingState } from "../sorting";
import { PagingState, PagingError } from "../paging";

describe('Dataset', () => {

    let dataGenerator: DataGenerator;
    let data: object[];
    let dataset: Dataset;

    beforeEach(() => {
        dataGenerator = new DataGenerator();
        data = dataGenerator.data;
        dataset = new Dataset(data);
    });

    it('should be defined', () => {
        dataset.configuration = {
            filtering: {
                tree: {
                    conditions: [
                        {
                            evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.GreaterThan],
                            key: 'number',
                            comparisonValue: 1
                        }
                    ],
                    operator: FilteringOperator.And
                }
            },
            sorting: {
                criteria: [
                    {
                        key: "string",
                        direction: SortDirection.DESC
                    }
                ]
            },
            paging: {
                currentPage: 0,
                itemsPerPage: 2
            }
        };

        dataset.process();
        expect(dataset.transformedData.map((d: any) => d.number)).toEqual([4, 3]);
    });

    it("tests process", () => {
        // test filtering
        dataset.configuration = {
            filtering: {
                tree: {
                    conditions: [
                        {
                            evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.GreaterThanOrEqualTo],
                            key: 'number',
                            comparisonValue: 1
                        }
                    ],
                    operator: FilteringOperator.And,
                }
            }
        };
        dataset.process();
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([1, 2, 3, 4]);
        expect(dataset.rawData.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3, 4]);
        // apply sorting without removing filtering
        dataset.configuration.sorting = {
            criteria: [
                {
                    key: "number",
                    direction: SortDirection.DESC
                }
            ]
        };
        dataset.process();
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([4, 3, 2, 1]);
        expect(dataset.rawData.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3, 4]);
        // apply paging(+filtering and sorting)
        dataset.configuration.paging = {
            currentPage: 1,
            itemsPerPage: 3
        };
        dataset.process();
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([1]);
        expect(dataset.configuration.paging.details.totalPages)
            .toEqual(2);
    });

    it("tests sort", () => {
        const sortingState: SortingState = {
            criteria: [
                {
                    key: "number",
                    direction: SortDirection.DESC
                }
            ]
        };
        dataset.process({ sorting: sortingState });
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([4, 3, 2, 1, 0]);
        expect(dataset.configuration.sorting)
            .toEqual(sortingState);
    });

    it("tests filter", () => {
        const filteringState: FilteringState = {
            tree: {
                conditions: [
                    {
                        evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.DoesNotEqual],
                        key: "number",
                        comparisonValue: 4
                    }
                ],
                operator: FilteringOperator.And
            }
        };
        dataset.process({ filtering: filteringState });
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3]);
        expect(dataset.configuration.filtering)
            .toEqual(filteringState);
    });

    it("tests page", () => {
        // apply sorting without removing filtering
        const pagingState: PagingState = {
            currentPage: 0,
            itemsPerPage: 4
        };
        dataset.process({ paging: pagingState });
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([0, 1, 2, 3]);
        expect(dataset.configuration.paging.details.totalPages)
            .toEqual(2);
        expect(dataset.configuration.paging.details.errorType)
            .toEqual(PagingError.None);
        pagingState.currentPage = 1;
        dataset.process({ paging: pagingState });
        expect(dataset.transformedData.map((d: any) => d.number))
            .toEqual([4]);
        expect(dataset.configuration.paging.details.totalPages)
            .toEqual(2);
        expect(dataset.configuration.paging.details.errorType)
            .toEqual(PagingError.None);
    });

    // test CRUD operations
    it("tests `insertRecord`", () => {
        let record = {
            number: -1
        };
        dataset.insertRecord(record);
        expect(dataset.rawData).toBeTruthy();
        expect(dataset.rawData.length).toBe(6);
        expect(dataset.rawData[5]).toEqual(record);
        // add at specific position
        record = { number: -2 };
        dataset.insertRecord(record, 0);
        expect(dataset.rawData.length).toBe(7);
        expect(dataset.rawData[0]).toEqual(record);
    });

    it("tests `removeRecord`", () => {
        const record = data[0];
        // remove first element
        const res = dataset.removeRecord(record);
        expect(res).toBeTruthy();
        expect(dataset.rawData.length).toBe(4);
        expect(dataset.rawData.map((d: any) => d.number))
      .toEqual([1, 2, 3, 4]);
    });

    it("tests `removeRecordAt`", () => {
        // remove first element
        const res = dataset.removeRecordAt(0);
        expect(res).toBeTruthy();
        expect(dataset.rawData.length).toBe(4);
        expect(dataset.rawData.map((d: any) => d.number))
            .toEqual([1, 2, 3, 4]);
    });

    it("tests `modifyRecordAt`", () => {
        const recordCopy = Object.assign({}, data[0]);
        dataset.modifyRecordAt(0, { number: -1 });
        (recordCopy as { number: number }).number = -1;
        expect(dataset.rawData[0]).toEqual(recordCopy);
    });

    it("tests `findRecordByField`", () => {
        let recordInfo = dataset.findRecordByField("number", 0);
        expect(recordInfo.position === 0 && recordInfo.data === dataset.rawData[0])
            .toBeTruthy();
        recordInfo = dataset.findRecordByField("number", -1);
        expect(recordInfo.position === -1 && recordInfo.data === undefined)
            .toBeTruthy();
    });

    it("tests `findRecordIndex`", () => {
        let index = dataset.findRecordIndex(data[1]);
        expect(index).toBe(1);
        index = dataset.findRecordIndex(data[0], DataSource.Processed);
        expect(index).toBe(0);
    });

    it("tests `getRecordAt`", () => {
        let rec = dataset.getRecordAt(0);
        expect(rec).toBe(data[0]);
        rec = dataset.getRecordAt(0, DataSource.Processed);
        expect(rec).toBe(dataset.transformedData[0]);
    });
});
