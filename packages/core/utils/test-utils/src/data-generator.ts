export class DataGenerator
{
    columns: IDataField[] = [];
    data: any[]           = [];

    constructor(countRows = 5, countCols = 4)
    {
        this.columns = this.generateFields(countCols);
        this.data    = this.generateData(countRows);
    }

    private generateFields(countCols: number): IDataField[]
    {
        let i: number;
        const defaultColumns: IDataField[] = [
            {
                field: 'number',
                type     : FieldDataType.Number,
            },
            {
                field: 'string',
                type     : FieldDataType.String,
            },
            {
                field: 'date',
                type     : FieldDataType.Date,
            },
            {
                field: 'boolean',
                type     : FieldDataType.Boolean,
            },
        ];
        if (countCols <= 0)
        {
            return defaultColumns;
        }
        if (countCols <= defaultColumns.length)
        {
            return defaultColumns.slice(0, countCols);
        }
        const len = countCols - defaultColumns.length;
        const res = defaultColumns;
        for (i = 0; i < len; i++)
        {
            res.push({
                field: `col${i}`,
                type     : FieldDataType.String,
            });
        }
        return res;
    }

    private generateData(countRows: number): any[]
    {
        let i, j, rec, val, col;
        const data = [];
        for (i = 0; i < countRows; i++)
        {
            rec = {};
            for (j = 0; j < this.columns.length; j++)
            {
                col = this.columns[j];
                switch (col.type)
                {
                    case FieldDataType.Number:
                        val = i;
                        break;
                    case FieldDataType.Date:
                        val = new Date(Date.now() + i * 24 * 60 * 60 * 1000);
                        break;
                    case FieldDataType.Boolean:
                        val = !!(i % 2);
                        break;
                    default:
                        val = `row${i}, col${j}`;
                        break;
                }
                rec[col.field] = val;
            }
            data.push(rec);
        }
        return data;
    }
}

export const FieldDataType = {
    String  : 'string',
    Number  : 'number',
    Boolean : 'boolean',
    Date    : 'date'
};
export type FieldDataType = (typeof FieldDataType)[keyof typeof FieldDataType];

export interface IDataField
{
    field: string;
    type: FieldDataType;
}
