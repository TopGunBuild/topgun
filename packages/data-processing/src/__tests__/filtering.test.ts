import { DataGenerator } from '@topgunbuild/test-utils';
import { DataFilteringEngine } from '../filtering/engine';
import { FilteringOperator, FilteringCriteriaTree } from '../filtering/types';
import { 
    BOOLEAN_FILTER_CONDITIONS, 
    NUMBER_FILTER_CONDITIONS, 
    NumberCondition, 
    STRING_FILTER_CONDITIONS, 
    StringCondition,
    BooleanCondition
} from '../filtering';


describe('Unit testing FilteringStrategy', () => {
    let dataGenerator: DataGenerator;
    let data: object[];
    let fs: DataFilteringEngine;
    
    beforeEach(() => {
        dataGenerator = new DataGenerator();
        data = dataGenerator.data;
        fs = new DataFilteringEngine();
    });

    it ('tests `filter`', () => {
        const expressionTree: FilteringCriteriaTree = {
            operator: FilteringOperator.And,
            conditions: [
                {
                    evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.GreaterThan],
                    key: 'number',
                    comparisonValue: 1
                },
            ],
        };
        const res = fs.process(data, expressionTree);
        expect(res.map((d: any) => d.number)).toEqual([2, 3, 4]);
    });

    it ('tests `matchRecordByExpressions`', () => {
        const rec = data[0];
        const expressionTree: FilteringCriteriaTree = {
            operator: FilteringOperator.Or,
            conditions: [
                {
                    evaluator: STRING_FILTER_CONDITIONS[StringCondition.Contains],
                    key: 'string',
                    comparisonValue: 'ROW'
                },
                {
                    evaluator: NUMBER_FILTER_CONDITIONS[NumberCondition.LessThan],
                    key: 'number',
                    comparisonValue: 1
                }
            ]
        };
        const res = fs.matchRecord(rec, expressionTree);
        expect(res).toBeTruthy();
    });

    it ('tests `matchByCriteria`', () => {
        const rec = data[0];
        const res = fs.matchByCriteria(rec, {
            evaluator: BOOLEAN_FILTER_CONDITIONS[BooleanCondition.False],
            key: 'boolean'
        });
        expect(res).toBeTruthy();
    });

    it ('tests default settings', () => {
        (data[0] as { string: string }).string = 'ROW';
        const filterstr = new DataFilteringEngine();
        const expressionTree: FilteringCriteriaTree = {
            operator: FilteringOperator.And,
            conditions: [
                {
                    evaluator: STRING_FILTER_CONDITIONS[StringCondition.Contains],
                    key: 'string',
                    comparisonValue: 'ROW'
                },
            ],
        };
        const res = filterstr.process(data, expressionTree);
        expect(res.map((d: any) => d.number)).toEqual([0]);
    });
});
