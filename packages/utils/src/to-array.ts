export const toArray = <T>(arr: T|T[]|undefined): T[] =>
{
    if (Array.isArray(arr))
    {
        return arr;
    }
    else if (arr)
    {
        return [arr];
    }
    else
    {
        return [];
    }
};
