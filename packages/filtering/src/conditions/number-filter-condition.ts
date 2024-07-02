import { FilterCondition } from './filter-condition';

export enum NumberCondition
{
    equals,
    doesNotEqual,
    greaterThan,
    lessThan,
    greaterThanOrEqualTo,
    lessThanOrEqualTo,
    empty,
    notEmpty
}

export class NumberFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : NumberCondition.equals,
                logic: (target: number, searchVal: number) =>
                {
                    return target === searchVal;
                },
            }, {
                name : NumberCondition.doesNotEqual,
                logic: (target: number, searchVal: number) =>
                {
                    return target !== searchVal;
                },
            }, {
                name : NumberCondition.greaterThan,
                logic: (target: number, searchVal: number) =>
                {
                    return target > searchVal;
                },
            }, {
                name : NumberCondition.lessThan,
                logic: (target: number, searchVal: number) =>
                {
                    return target < searchVal;
                },
            }, {
                name : NumberCondition.greaterThanOrEqualTo,
                logic: (target: number, searchVal: number) =>
                {
                    return target >= searchVal;
                },
            }, {
                name : NumberCondition.lessThanOrEqualTo,
                logic: (target: number, searchVal: number) =>
                {
                    return target <= searchVal;
                },
            }, {
                name : NumberCondition.empty,
                logic: (target: number) =>
                {
                    return target === null || target === undefined || isNaN(target);
                },
            }, {
                name : NumberCondition.notEmpty,
                logic: (target: number) =>
                {
                    return target !== null && target !== undefined && !isNaN(target);
                },
            },
        );
    }
}
