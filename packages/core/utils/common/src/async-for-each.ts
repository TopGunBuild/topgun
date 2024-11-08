export async function asyncForEach<T extends any>(array: T[], callback: (item: T, i: number, arr: T[]) => any): Promise<void>
{
    array = Array.isArray(array) ? array : [];
    for (let index = 0; index < array.length; index++)
    {
        await callback(array[index], index, array);
    }
}
