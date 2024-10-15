import {
    And, Or, StringConditionQuery, NumberConditionQuery,
    SelectMessagesAction} from '@topgunbuild/types';
import { convertSelectToFilterExpressionTree } from '../utils/convert-select';

describe('convertSelectToFilterExpressionTree', () => {
    it('should return an empty FilterExpressionTree when select.query is not an array', () => {
        const select: SelectMessagesAction = { query: null } as any;
        const result = convertSelectToFilterExpressionTree(select);
        expect(result.conditions).toEqual([]);
    });

    it('should convert a simple FieldQuery to a FilterExpression', () => {
        const query: StringConditionQuery = new StringConditionQuery({ key: 'key', condition: 0, value: 'value', caseInsensitive: false });
        const select: SelectMessagesAction = { query: [query] } as any;
        const result = convertSelectToFilterExpressionTree(select);
        expect(result.conditions.length).toBe(1);
    });

    it('should handle And logical queries', () => {
        const subQuery1: NumberConditionQuery = new NumberConditionQuery({ key: 'key1', condition: 0, value: 10 });
        const subQuery2: NumberConditionQuery = new NumberConditionQuery({ key: 'key2', condition: 0, value: 20 });
        const andQuery: And = new And([subQuery1, subQuery2]);
        const select: SelectMessagesAction = { query: [andQuery] } as any;
        const result = convertSelectToFilterExpressionTree(select);
        expect(result.conditions.length).toBe(1);
        // expect(result.conditions[0]).toBeInstanceOf(FilteringCriteriaTree);
    });

    it('should handle Or logical queries', () => {
        const subQuery1: NumberConditionQuery = new NumberConditionQuery({ key: 'key1', condition: 0, value: 10 });
        const subQuery2: NumberConditionQuery = new NumberConditionQuery({ key: 'key2', condition: 0, value: 20 });
        const orQuery: Or = new Or([subQuery1, subQuery2]);
        const select: SelectMessagesAction = { query: [orQuery] } as any;
        const result = convertSelectToFilterExpressionTree(select);
        expect(result.conditions.length).toBe(1);
        // expect(result.conditions[0]).toBeInstanceOf(FilteringCriteriaTree);
    });
});