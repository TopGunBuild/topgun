import { FilterDataType } from './data-type';

export interface FilterElement
{
    name: string;
    logic?: (value: any, searchVal?: any, ignoreCase?: boolean) => boolean;
    type?: FilterDataType;
}
