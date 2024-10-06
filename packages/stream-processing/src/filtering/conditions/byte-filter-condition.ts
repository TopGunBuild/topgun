import { equalBytes } from '@topgunbuild/utils';
import { FilterCondition } from './filter-condition';

export enum ByteCondition
{
    equals,
    doesNotEqual,
    empty,
    notEmpty
}

export class ByteFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : ByteCondition.equals,
                logic: (target: Uint8Array, searchVal: Uint8Array) =>
                {
                    return equalBytes(target, searchVal);
                },
            }, {
                name : ByteCondition.doesNotEqual,
                logic: (target: Uint8Array, searchVal: Uint8Array) =>
                {
                    return !equalBytes(target, searchVal);
                },
            }, {
                name : ByteCondition.empty,
                logic: (target: boolean) =>
                {
                    return target === null || target === undefined;
                },
            }, {
                name : ByteCondition.notEmpty,
                logic: (target: boolean) =>
                {
                    return target !== null && target !== undefined;
                },
            },
        );
    }
}
