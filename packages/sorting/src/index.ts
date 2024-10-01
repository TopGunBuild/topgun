import { isNil } from '@topgunbuild/utils';

export enum SortDirection
{
    None,
    Asc,
    Desc
}

export class SortConfig
{
    name: string;
    dir: SortDirection;
    caseInsensitive?: boolean;

    constructor(name: string, dir: SortDirection, caseInsensitive?: boolean)
    {
        this.name            = name;
        this.dir             = dir;
        this.caseInsensitive = caseInsensitive;
    }
}

export class SortService
{
    sort(data: any[], configs: SortConfig[]): any[]
    {
        return this.sortDataRecursive(data, configs);
    }

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

    private arraySort<T>(data: T[], compareFn: (a: T, b: T) => number): T[]
    {
        return data.sort(compareFn);
    }

    private groupedRecordsByExpression<T>(data: T[], index: number, expression: SortConfig): T[]
    {
        const res: T[] = [];
        const key = expression.name;
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

    private sortByFieldExpression<T>(data: T[], expression: SortConfig): T[]
    {
        const key = expression.name;
        const caseInsensitive = expression.caseInsensitive != null && expression.caseInsensitive &&
            data.length > 0 && typeof data[0] === 'object' &&
            (data[0][key] == null || typeof data[0][key] === 'string');
        const reverse = (expression.dir === SortDirection.Desc ? -1 : 1);
        const cmpFunc = (obj1: T, obj2: T) => this.compareObjects(obj1 as Record<string, any>, obj2 as Record<string, any>, key, reverse, caseInsensitive);
        return this.arraySort(data, cmpFunc);
    }

    private sortDataRecursive<T>(data: T[], configs: SortConfig[], expressionIndex: number = 0): T[]
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

