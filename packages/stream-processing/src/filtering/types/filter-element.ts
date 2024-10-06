export interface FilterElement
{
    name: number;
    logic?: (value: any, searchVal?: any, caseInsensitive?: boolean) => boolean;
}
