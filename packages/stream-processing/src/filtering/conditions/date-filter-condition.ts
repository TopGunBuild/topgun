import { DateParts } from '../types/date-parts';
import { FilterCondition } from './filter-condition';

export enum DateCondition
{
    equals,
    doesNotEqual,
    before,
    after,
    today,
    yesterday,
    thisMonth,
    lastMonth,
    nextMonth,
    thisYear,
    lastYear,
    nextYear,
    empty,
    notEmpty
}

export class DateFilterCondition extends FilterCondition
{
    constructor()
    {
        super();
        this.elements.push(
            {
                name : DateCondition.equals,
                logic: (target: Date, searchVal: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const targetp = DateFilterCondition.getDateParts(target, 'yMd');
                    const searchp = DateFilterCondition.getDateParts(searchVal, 'yMd');
                    return targetp.year === searchp.year &&
                        targetp.month === searchp.month &&
                        targetp.day === searchp.day;
                },
            },
            {
                name : DateCondition.doesNotEqual,
                logic: (target: Date, searchVal: Date) =>
                {
                    if (!target)
                    {
                        return true;
                    }

                    this.validateInputData(target);

                    const targetp = DateFilterCondition.getDateParts(target, 'yMd');
                    const searchp = DateFilterCondition.getDateParts(searchVal, 'yMd');
                    return targetp.year !== searchp.year ||
                        targetp.month !== searchp.month ||
                        targetp.day !== searchp.day;
                },
            },
            {
                name : DateCondition.before,
                logic: (target: Date, searchVal: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    return target < searchVal;
                },
            },
            {
                name : DateCondition.after,
                logic: (target: Date, searchVal: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    return target > searchVal;
                },
            },
            {
                name : DateCondition.today,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'yMd');
                    const now = DateFilterCondition.getDateParts(new Date(), 'yMd');
                    return d.year === now.year &&
                        d.month === now.month &&
                        d.day === now.day;
                },
            },
            {
                name : DateCondition.yesterday,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const td        = DateFilterCondition.getDateParts(target, 'yMd');
                    const y         = ((d) => new Date(d.setDate(d.getDate() - 1)))(new Date());
                    const yesterday = DateFilterCondition.getDateParts(y, 'yMd');
                    return td.year === yesterday.year &&
                        td.month === yesterday.month &&
                        td.day === yesterday.day;
                },
            },
            {
                name : DateCondition.thisMonth,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'yM');
                    const now = DateFilterCondition.getDateParts(new Date(), 'yM');
                    return d.year === now.year &&
                        d.month === now.month;
                },
            },
            {
                name : DateCondition.lastMonth,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'yM');
                    const now = DateFilterCondition.getDateParts(new Date(), 'yM');
                    if (!now.month)
                    {
                        now.month = 11;
                        now.year -= 1;
                    }
                    else
                    {
                        now.month--;
                    }
                    return d.year === now.year &&
                        d.month === now.month;
                },
            },
            {
                name : DateCondition.nextMonth,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'yM');
                    const now = DateFilterCondition.getDateParts(new Date(), 'yM');
                    if (now.month === 11)
                    {
                        now.month = 0;
                        now.year += 1;
                    }
                    else
                    {
                        now.month++;
                    }
                    return d.year === now.year &&
                        d.month === now.month;
                },
            },
            {
                name : DateCondition.thisYear,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'y');
                    const now = DateFilterCondition.getDateParts(new Date(), 'y');
                    return d.year === now.year;
                },
            },
            {
                name : DateCondition.lastYear,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'y');
                    const now = DateFilterCondition.getDateParts(new Date(), 'y');
                    return d.year === now.year - 1;
                },
            },
            {
                name : DateCondition.nextYear,
                logic: (target: Date) =>
                {
                    if (!target)
                    {
                        return false;
                    }

                    this.validateInputData(target);

                    const d   = DateFilterCondition.getDateParts(target, 'y');
                    const now = DateFilterCondition.getDateParts(new Date(), 'y');
                    return d.year === now.year + 1;
                },
            },
            {
                name : DateCondition.empty,
                logic: (target: Date) =>
                {
                    return target === null || target === undefined;
                },
            },
            {
                name : DateCondition.notEmpty,
                logic: (target: Date) =>
                {
                    return target !== null && target !== undefined;
                },
            },
        );
    }

    static getDateParts(date: Date|string, dateFormat?: string): DateParts
    {
        date      = new Date(date);
        const res = {
            day         : null,
            hours       : null,
            milliseconds: null,
            minutes     : null,
            month       : null,
            seconds     : null,
            year        : null,
        };
        if (!date || !dateFormat)
        {
            return res;
        }
        if (dateFormat.indexOf('y') >= 0)
        {
            res.year = date.getFullYear();
        }
        if (dateFormat.indexOf('M') >= 0)
        {
            res.month = date.getMonth();
        }
        if (dateFormat.indexOf('d') >= 0)
        {
            res.day = date.getDate();
        }
        if (dateFormat.indexOf('h') >= 0)
        {
            res.hours = date.getHours();
        }
        if (dateFormat.indexOf('m') >= 0)
        {
            res.minutes = date.getMinutes();
        }
        if (dateFormat.indexOf('s') >= 0)
        {
            res.seconds = date.getSeconds();
        }
        if (dateFormat.indexOf('f') >= 0)
        {
            res.milliseconds = date.getMilliseconds();
        }
        return res;
    }

    private isDate(date: any): boolean
    {
        return !isNaN(Date.parse(date));
    }

    private validateInputData(target: Date): boolean
    {
        if (this.isDate(target))
        {
            return true;
        }
        if (!(target instanceof Date))
        {
            throw new Error('Could not perform filtering on \'date\' column because the datasource object type is not \'Date\'.');
        }

        return false;
    }
}
