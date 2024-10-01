import { DataGenerator } from '@topgunbuild/test-utils';
import {
    BooleanCondition,
    BooleanFilterCondition,
    FilterExpressionTree,
    FilterLogic,
    FilterService, NumberCondition,
    NumberFilterCondition, StringCondition,
    StringFilterCondition,
} from '..';

describe('Unit testing Filtering', () =>
{
    let dataGenerator: DataGenerator;
    let data: any[];
    let fs: FilterService;

    beforeEach(() =>
    {
        dataGenerator = new DataGenerator(10);
        data          = dataGenerator.data;
        fs            = new FilterService();
    });

    it('tests `matchRecord`', () =>
    {
        const rec               = data[0];
        const filterChain       = new FilterExpressionTree(FilterLogic.Or);
        filterChain.expressions = [
            {
                condition      : new StringFilterCondition().condition(StringCondition.contains),
                key            : 'string',
                caseInsensitive: false,
                value          : 'ROW',
            },
            {
                condition: new NumberFilterCondition().condition(NumberCondition.lessThan),
                key      : 'number',
                value    : 1,
            },
        ];
        const res               = fs.matchRecord(rec, filterChain);
        expect(res).toBeTruthy();
    });

    it('tests `findMatch`', () =>
    {
        const rec = data[0];
        const res = fs.matchByExpression(rec, {
            condition: new BooleanFilterCondition().condition(BooleanCondition.false),
            key      : 'boolean',
        });
        expect(res).toBeTruthy();
    });

    it('tests `applyOnlyToKey`', () =>
    {
        const rec               = data[0];
        const filterChain       = new FilterExpressionTree(FilterLogic.And);
        filterChain.expressions = [
            {
                condition      : new StringFilterCondition().condition(StringCondition.contains),
                key            : 'string',
                caseInsensitive: false,
                value          : 'ROW',
            },
            {
                condition: new NumberFilterCondition().condition(NumberCondition.lessThan),
                key      : 'number',
                value    : 1,
            },
        ];
        const res               = fs.matchRecord(rec, filterChain, 'number');
        expect(res).toBeTruthy();
    });
});

