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
import { OrderDirection, OrderService } from '../sort';
import * as console from 'node:console';

const chunk = (arr, size) =>
    Array.from({ length: Math.ceil(arr.length / size) }, (v, i) =>
        arr.slice(i * size, i * size + size),
    );

describe('Unit testing Filtering', () =>
{
    let dataGenerator: DataGenerator;
    let data: any[];
    let fs: FilterService;
    let os: OrderService;

    beforeEach(() =>
    {
        dataGenerator = new DataGenerator(10);
        data          = dataGenerator.data;
        fs            = new FilterService();
        os            = new OrderService();
    });

    it('should filter', () =>
    {
        const db       = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        const arr      = [5, 6, 7, 8, 9, 10];
        const getFirst = (_arr: any[]) => _arr[0];
        const getLast  = (_arr: any[]) => _arr[_arr.length - 1];

        // select * from db limit 5 offset 5

        // console.log({
        //     first: getFirst(),
        //     last : getLast(),
        // });

        expect(data.length > 0).toBeTruthy();
    });

    it('tests `sort`', () =>
    {
        const res = os.order(data, [
            {
                dir : OrderDirection.Asc,
                name: 'boolean',
            }, {
                dir : OrderDirection.Desc,
                name: 'number',
            }]);

        expect(res.map(item => item.number)).toEqual([8, 6, 4, 2, 0, 9, 7, 5, 3, 1]);
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

