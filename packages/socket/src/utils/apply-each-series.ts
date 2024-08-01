import { asyncForEach } from '@topgunbuild/utils';

export type AsyncFunction<T = any> = (...args: any[]) => T|Promise<T>;

export async function applyEachSeries<T extends AsyncFunction[]>(tasks: T, ...args: Parameters<T[number]>): Promise<Array<ReturnType<T[number]>>>
{
    const callback = typeof args[args.length - 1] === 'function' ? args.pop() : () =>
    {
    };
    let err        = null;
    const results  = [];

    await asyncForEach(tasks, async (task) =>
    {
        if (!err)
        {
            try
            {
                const result = await task(...args);
                results.push(result);
            }
            catch (e)
            {
                results.push(undefined);
                err = e;
            }
        }
    });

    callback(err, results);
    return results;
}
