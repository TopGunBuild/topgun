import { equalBytes } from '@topgunbuild/common';
import { extractDateComponents, applyIgnoreCase } from './utils';

/**
 * Filter conditions for boolean data.
 */
export enum BooleanCondition {
    True,
    False,
    Null,
    NotNull,
    Empty,
    NotEmpty
}

/**
 * Filter conditions for date data.
 */
export enum DateCondition {
    Equals,
    DoesNotEqual,
    Before,
    After,
    Today,
    Yesterday,
    ThisMonth,
    LastMonth,
    NextMonth,
    ThisYear,
    LastYear,
    NextYear,
    Null,
    NotNull,
    Empty,
    NotEmpty
}

/**
 * Filter conditions for number data.
 */
export enum NumberCondition {
    Equals,
    DoesNotEqual,
    GreaterThan,
    LessThan,
    GreaterThanOrEqualTo,
    LessThanOrEqualTo,
    Null,
    NotNull,
    Empty,
    NotEmpty
}

/**
 * Filter conditions for string data.
 */
export enum StringCondition {
    Contains,
    StartsWith,
    EndsWith,
    DoesNotContain,
    Equals,
    DoesNotEqual,
    Null,
    NotNull,
    Empty,
    NotEmpty
}

/**
 * Filter conditions for byte data.
 */
export enum ByteCondition {
    Equals,
    DoesNotEqual,
    Null,
    NotNull,
    Empty,
    NotEmpty
}

/**
 * Filter conditions for byte data.
 */
export const BYTE_FILTER_CONDITIONS = {
    [ByteCondition.Equals]: (target: Uint8Array, searchVal: Uint8Array): boolean => equalBytes(target, searchVal),
    [ByteCondition.DoesNotEqual]: (target: Uint8Array, searchVal: Uint8Array): boolean => !equalBytes(target, searchVal),
    [ByteCondition.Null]: (target: Uint8Array): boolean => target === null,
    [ByteCondition.NotNull]: (target: Uint8Array): boolean => target !== null,
    [ByteCondition.Empty]: (target: Uint8Array): boolean => target === null || target === undefined,
    [ByteCondition.NotEmpty]: (target: Uint8Array): boolean => target !== null && target !== undefined
};

/**
 * Filter conditions for boolean data.
 */
export const BOOLEAN_FILTER_CONDITIONS = {
    [BooleanCondition.True]: (target: boolean): boolean => target,
    [BooleanCondition.False]: (target: boolean): boolean => !target,
    [BooleanCondition.Null]: (target: boolean): boolean => target === null,
    [BooleanCondition.NotNull]: (target: boolean): boolean => target !== null,
    [BooleanCondition.Empty]: (target: boolean): boolean => target === null || target === undefined,
    [BooleanCondition.NotEmpty]: (target: boolean): boolean => target !== null && target !== undefined
};

/**
 * Filter conditions for date data.
 */
export const DATE_FILTER_CONDITIONS = {
    [DateCondition.Equals]: (target: Date, searchVal: Date): boolean => +target === +searchVal,
    [DateCondition.DoesNotEqual]: (target: Date, searchVal: Date): boolean => +target !== +searchVal,
    [DateCondition.Before]: (target: Date, searchVal: Date): boolean => target < searchVal,
    [DateCondition.After]: (target: Date, searchVal: Date): boolean => target > searchVal,
    [DateCondition.Today]: (target: Date): boolean => {
        const d = extractDateComponents(target, "yMd");
        const now = extractDateComponents(new Date(), "yMd");
        return d.year === now.year && d.month === now.month && d.day === now.day;
    },
    [DateCondition.Yesterday]: (target: Date): boolean => {
        const td = extractDateComponents(target, "yMd");
        const y = ((d) => new Date(d.setDate(d.getDate() - 1)))(new Date());
        const yesterday = extractDateComponents(y, "yMd");
        return td.year === yesterday.year && td.month === yesterday.month && td.day === yesterday.day;
    },
    [DateCondition.ThisMonth]: (target: Date): boolean => {
        const d = extractDateComponents(target, "yM");
        const now = extractDateComponents(new Date(), "yM");
        return d.year === now.year && d.month === now.month;
    },
    [DateCondition.LastMonth]: (target: Date): boolean => {
        const d = extractDateComponents(target, "yM");
        const now = extractDateComponents(new Date(), "yM");
        if (!now.month) {
            now.month = 11;
            now.year -= 1;
        } else {
            now.month--;
        }
        return d.year === now.year && d.month === now.month;
    },
    [DateCondition.NextMonth]: (target: Date): boolean => {
        const d = extractDateComponents(target, "yM");
        const now = extractDateComponents(new Date(), "yM");
        if (now.month === 11) {
            now.month = 0;
            now.year += 1;
        } else {
            now.month++;
        }
        return d.year === now.year && d.month === now.month;
    },
    [DateCondition.ThisYear]: (target: Date): boolean => {
        const d = extractDateComponents(target, "y");
        const now = extractDateComponents(new Date(), "y");
        return d.year === now.year;
    },
    [DateCondition.LastYear]: (target: Date): boolean => {
        const d = extractDateComponents(target, "y");
        const now = extractDateComponents(new Date(), "y");
        return d.year === now.year - 1;
    },
    [DateCondition.NextYear]: (target: Date): boolean => {
        const d = extractDateComponents(target, "y");
        const now = extractDateComponents(new Date(), "y");
        return d.year === now.year + 1;
    },
    [DateCondition.Null]: (target: Date): boolean => target === null,
    [DateCondition.NotNull]: (target: Date): boolean => target !== null,
    [DateCondition.Empty]: (target: Date): boolean => target === null || target === undefined,
    [DateCondition.NotEmpty]: (target: Date): boolean => target !== null && target !== undefined
};

/**
 * Filter conditions for number data.
 */
export const NUMBER_FILTER_CONDITIONS = {
    [NumberCondition.Equals]: (target: number, searchVal: number): boolean => target === searchVal,
    [NumberCondition.DoesNotEqual]: (target: number, searchVal: number): boolean => target !== searchVal,
    [NumberCondition.GreaterThan]: (target: number, searchVal: number): boolean => target > searchVal,
    [NumberCondition.LessThan]: (target: number, searchVal: number): boolean => target < searchVal,
    [NumberCondition.GreaterThanOrEqualTo]: (target: number, searchVal: number): boolean => target >= searchVal,
    [NumberCondition.LessThanOrEqualTo]: (target: number, searchVal: number): boolean => target <= searchVal,
    [NumberCondition.Null]: (target: number): boolean => target === null,
    [NumberCondition.NotNull]: (target: number): boolean => target !== null,
    [NumberCondition.Empty]: (target: number): boolean => target === null || target === undefined || isNaN(target),
    [NumberCondition.NotEmpty]: (target: number): boolean => target !== null && target !== undefined && !isNaN(target)
};

/**
 * Filter conditions for string data.
 */
export const STRING_FILTER_CONDITIONS = {
    [StringCondition.Contains]: (target: string, searchVal: string, ignoreCase?: boolean): boolean => {
        const search = applyIgnoreCase(searchVal, ignoreCase);
        target = applyIgnoreCase(target, ignoreCase);
        return target.indexOf(search) !== -1;
    },
    [StringCondition.StartsWith]: (target: string, searchVal: string, ignoreCase?: boolean): boolean => {
        const search = applyIgnoreCase(searchVal, ignoreCase);
        target = applyIgnoreCase(target, ignoreCase);
        return target.startsWith(search);
    },
    [StringCondition.EndsWith]: (target: string, searchVal: string, ignoreCase?: boolean): boolean => {
        const search = applyIgnoreCase(searchVal, ignoreCase);
        target = applyIgnoreCase(target, ignoreCase);
        return target.endsWith(search);
    },
    [StringCondition.DoesNotContain]: (target: string, searchVal: string, ignoreCase?: boolean): boolean => {
        const search = applyIgnoreCase(searchVal, ignoreCase);
        target = applyIgnoreCase(target, ignoreCase);
        return target.indexOf(search) === -1;
    },
    [StringCondition.Equals]: (target: string, searchVal: string, ignoreCase?: boolean): boolean => {
        const search = applyIgnoreCase(searchVal, ignoreCase);
        target = applyIgnoreCase(target, ignoreCase);
        return target === search;
    },
    [StringCondition.DoesNotEqual]: (target: string, searchVal: string, ignoreCase?: boolean): boolean => {
        const search = applyIgnoreCase(searchVal, ignoreCase);
        target = applyIgnoreCase(target, ignoreCase);
        return target !== search;
    },
    [StringCondition.Null]: (target: string): boolean => target === null,
    [StringCondition.NotNull]: (target: string): boolean => target !== null,
    [StringCondition.Empty]: (target: string): boolean => target === null || target === undefined || target.length === 0,
    [StringCondition.NotEmpty]: (target: string): boolean => target !== null && target !== undefined && target.length > 0
};

// Combine all filter conditions
export const FILTER_CONDITIONS = {
    boolean: BOOLEAN_FILTER_CONDITIONS,
    date: DATE_FILTER_CONDITIONS,
    number: NUMBER_FILTER_CONDITIONS,
    string: STRING_FILTER_CONDITIONS,
    byte: BYTE_FILTER_CONDITIONS
};
