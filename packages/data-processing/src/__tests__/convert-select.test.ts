import {
    And, Or, StringConditionQuery, NumberConditionQuery,
    SelectRequest
} from '@topgunbuild/transport';
import { convertQueryToFilterTree } from '../data-frame/utils';

describe('convertSelectToFilterExpressionTree', () => {
    it('should return an empty FilterExpressionTree when select.query is not an array', () => {
        const select: SelectRequest = { query: null } as any;
        const result = convertQueryToFilterTree(select);
        expect(result.conditions).toEqual([]);
    });

    it('should convert a simple FieldQuery to a FilterExpression', () => {
        const query: StringConditionQuery = new StringConditionQuery({ 
            key: 'key', 
            condition: 0, 
            value: 'value', 
            caseInsensitive: false 
        });
        const select: SelectRequest = { query: [query] } as any;
        const result = convertQueryToFilterTree(select);
        expect(result.conditions.length).toBe(1);
    });

    it('should handle And logical queries', () => {
        const subQuery1: NumberConditionQuery = new NumberConditionQuery({ key: 'key1', condition: 0, value: 10 });
        const subQuery2: NumberConditionQuery = new NumberConditionQuery({ key: 'key2', condition: 0, value: 20 });
        const andQuery: And = new And([subQuery1, subQuery2]);
        const select: SelectRequest = { query: [andQuery] } as any;
        const result = convertQueryToFilterTree(select);
        expect(result.conditions.length).toBe(1);
        // expect(result.conditions[0]).toBeInstanceOf(FilteringCriteriaTree);
    });

    it('should handle Or logical queries', () => {
        const subQuery1: NumberConditionQuery = new NumberConditionQuery({ key: 'key1', condition: 0, value: 10 });
        const subQuery2: NumberConditionQuery = new NumberConditionQuery({ key: 'key2', condition: 0, value: 20 });
        const orQuery: Or = new Or([subQuery1, subQuery2]);
        const select: SelectRequest = { query: [orQuery] } as any;
        const result = convertQueryToFilterTree(select);
        expect(result.conditions.length).toBe(1);
        // expect(result.conditions[0]).toBeInstanceOf(FilteringCriteriaTree);
    });

    it('should handle empty query array', () => {
        const select: SelectRequest = { query: [] } as any;
        const result = convertQueryToFilterTree(select);
        expect(result.conditions).toEqual([]);
    });

    it('should handle nested And/Or queries', () => {
        const subQuery1 = new NumberConditionQuery({ key: 'key1', condition: 0, value: 10 });
        const subQuery2 = new NumberConditionQuery({ key: 'key2', condition: 0, value: 20 });
        const orQuery = new Or([subQuery1, subQuery2]);
        const andQuery = new And([orQuery, subQuery1]);
        const select: SelectRequest = { query: [andQuery] } as any;
        const result = convertQueryToFilterTree(select);
        expect(result.conditions.length).toBe(1);
        // Add more specific assertions about the nested structure
    });
});