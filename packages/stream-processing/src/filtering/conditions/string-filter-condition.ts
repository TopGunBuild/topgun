import { FilterCondition } from './filter-condition';

export enum StringCondition
{
    contains,
    doesNotContain,
    startsWith,
    endsWith,
    equals,
    doesNotEqual,
    empty,
    notEmpty
}

export class StringFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : StringCondition.contains,
                logic: (target: string, searchVal: string, caseInsensitive?: boolean) =>
                {
                    const search = StringFilterCondition.applyCaseInsensitive(searchVal, caseInsensitive);
                    target       = StringFilterCondition.applyCaseInsensitive(target, caseInsensitive);
                    return target.indexOf(search) !== -1;
                },
            }, {
                name : StringCondition.doesNotContain,
                logic: (target: string, searchVal: string, caseInsensitive?: boolean) =>
                {
                    const search = StringFilterCondition.applyCaseInsensitive(searchVal, caseInsensitive);
                    target       = StringFilterCondition.applyCaseInsensitive(target, caseInsensitive);
                    return target.indexOf(search) === -1;
                },
            }, {
                name : StringCondition.startsWith,
                logic: (target: string, searchVal: string, caseInsensitive?: boolean) =>
                {
                    const search = StringFilterCondition.applyCaseInsensitive(searchVal, caseInsensitive);
                    target       = StringFilterCondition.applyCaseInsensitive(target, caseInsensitive);
                    return target.startsWith(search);
                },
            }, {
                name : StringCondition.endsWith,
                logic: (target: string, searchVal: string, caseInsensitive?: boolean) =>
                {
                    const search = StringFilterCondition.applyCaseInsensitive(searchVal, caseInsensitive);
                    target       = StringFilterCondition.applyCaseInsensitive(target, caseInsensitive);
                    return target.endsWith(search);
                },
            }, {
                name : StringCondition.equals,
                logic: (target: string, searchVal: string, caseInsensitive?: boolean) =>
                {
                    const search = StringFilterCondition.applyCaseInsensitive(searchVal, caseInsensitive);
                    target       = StringFilterCondition.applyCaseInsensitive(target, caseInsensitive);
                    return target === search;
                },
            }, {
                name : StringCondition.doesNotEqual,
                logic: (target: string, searchVal: string, caseInsensitive?: boolean) =>
                {
                    const search = StringFilterCondition.applyCaseInsensitive(searchVal, caseInsensitive);
                    target       = StringFilterCondition.applyCaseInsensitive(target, caseInsensitive);
                    return target !== search;
                },
            }, {
                name : StringCondition.empty,
                logic: (target: string) =>
                {
                    return target === null || target === undefined || target.length === 0;
                },
            }, {
                name : StringCondition.notEmpty,
                logic: (target: string) =>
                {
                    return target !== null && target !== undefined && target.length > 0;
                },
            },
        );
    }

    static applyCaseInsensitive(a: string, caseInsensitive: boolean): string
    {
        a = a || '';
        return caseInsensitive ? ('' + a).toLowerCase() : a;
    }
}
