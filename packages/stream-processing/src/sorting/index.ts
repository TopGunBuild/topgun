import { isNil } from '@topgunbuild/utils';
import { SortDirection, SortParams } from '@topgunbuild/types';

/**
 * Class to hold sorting configuration for a specific field
 */
export interface SortingExpression extends SortParams
{
    caseInsensitive?: boolean;
}

export class SortingService
{
    /**
     * Method to sort data based on provided configurations
     * @param {any[]} data
     * @param {SortingExpression[]} configs
     * @returns {any[]}
     */
    sort(data: any[], configs: SortingExpression[]): any[]
    {
        return this.sortDataRecursive(data, configs);
    }

    /**
     * Method to compare two values
     * @param a
     * @param b
     * @returns {number}
     */
    compareValues(a: any, b: any): number
    {
        const aNil = isNil(a);
        const bNil = isNil(b);
        if (aNil)
        {
            if (bNil)
            {
                return 0;
            }
            return -1;
        }
        else if (bNil)
        {
            return 1;
        }
        return a > b ? 1 : a < b ? -1 : 0;
    }

    /**
     * Method to compare two objects based on a specific key
     * @param {Record<string, any>} obj1
     * @param {Record<string, any>} obj2
     * @param {string} key
     * @param {number} reverse
     * @param {boolean} caseInsensitive
     * @returns {number}
     * @private
     */
    private compareObjects(
        obj1: Record<string, any>,
        obj2: Record<string, any>,
        key: string,
        reverse: number,
        caseInsensitive: boolean
    ): number
    {
        let a = obj1[key];
        let b = obj2[key];
        if (caseInsensitive)
        {
            a = a && a.toLowerCase ? a.toLowerCase() : a;
            b = b && b.toLowerCase ? b.toLowerCase() : b;
        }
        return reverse * this.compareValues(a, b);
    }

    /**
     * Method to sort an array using a comparison function
     * @param {T[]} data
     * @param {(a: T, b: T) => number} compareFn
     * @returns {T[]}
     * @private
     */
    private arraySort<T>(data: T[], compareFn: (a: T, b: T) => number): T[]
    {
        return data.sort(compareFn);
    }

    /**
     * Method to group records by a specific expression
     * @param {T[]} data
     * @param {number} index
     * @param {SortingExpression} expression
     * @returns {T[]}
     * @private
     */
    private groupedRecordsByExpression<T>(data: T[], index: number, expression: SortingExpression): T[]
    {
        const res: T[] = [];
        const key = expression.key;
        const len = data.length;
        const getValue = (obj: any) => (obj as Record<string, any>)[key];

        res.push(data[index]);
        const groupVal = getValue(data[index]);
        index++;
        for (let i = index; i < len; i++) {
            if (getValue(data[i]) === groupVal) {
                res.push(data[i]);
            } else {
                break;
            }
        }
        return res;
    }

    /**
     * Method to sort data by a specific field expression
     * @param {T[]} data
     * @param {SortingExpression} expression
     * @returns {T[]}
     * @private
     */
    private sortByFieldExpression<T>(data: T[], expression: SortingExpression): T[]
    {
        const key = expression.key;
        const caseInsensitive = expression.caseInsensitive != null && expression.caseInsensitive &&
            data.length > 0 && typeof data[0] === 'object' &&
            (data[0][key] == null || typeof data[0][key] === 'string');
        const reverse = (expression.direction === SortDirection.DESC ? -1 : 1);
        const cmpFunc = (obj1: T, obj2: T) => this.compareObjects(obj1 as Record<string, any>, obj2 as Record<string, any>, key, reverse, caseInsensitive);
        return this.arraySort(data, cmpFunc);
    }

    /**
     * Recursive method to sort data based on multiple configurations
     * @param {T[]} data
     * @param {SortingExpression[]} configs
     * @param {number} expressionIndex
     * @returns {T[]}
     * @private
     */
    private sortDataRecursive<T>(data: T[], configs: SortingExpression[], expressionIndex: number = 0): T[]
    {
        const exprsLen = configs.length;
        const dataLen  = data.length;
        if (expressionIndex >= exprsLen || dataLen <= 1)
        {
            return data;
        }
        const expr = configs[expressionIndex];
        data       = this.sortByFieldExpression(data, expr);
        if (expressionIndex === exprsLen - 1)
        {
            return data;
        }
        // in case of multiple sorting
        const result: T[] = [...data];
        for (let i = 0; i < dataLen; i++)
        {
            let gbData      = this.groupedRecordsByExpression(data, i, expr);
            const gbDataLen = gbData.length;
            if (gbDataLen > 1)
            {
                gbData = this.sortDataRecursive(gbData, configs, expressionIndex + 1);
            }
            for (let j = 0; j < gbDataLen; j++)
            {
                result[i + j] = gbData[j];
            }
            i += gbDataLen - 1;
        }
        return result;
    }
}

