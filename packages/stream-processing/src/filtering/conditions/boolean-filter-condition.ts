import { FilterCondition } from './filter-condition';

export enum BooleanCondition
{
    true,
    false,
    empty,
    notEmpty
}

export class BooleanFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : BooleanCondition.true,
                logic: (target: boolean) =>
                {
                    return !!(target && target !== null && target !== undefined);
                },
            }, {
                name : BooleanCondition.false,
                logic: (target: boolean) =>
                {
                    return !target && target !== null && target !== undefined;
                },
            }, {
                name : BooleanCondition.empty,
                logic: (target: boolean) =>
                {
                    return target === null || target === undefined;
                },
            }, {
                name : BooleanCondition.notEmpty,
                logic: (target: boolean) =>
                {
                    return target !== null && target !== undefined;
                },
            },
        );
    }
}
