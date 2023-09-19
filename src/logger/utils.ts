export const asyncFunc = (cb: (...args: any[]) => any): void =>
{
    setTimeout(() =>
    {
        return cb();
    }, 0);
};

export const stringifyFunc = (msg: any): string =>
{
    let stringMsg = '';
    if (typeof msg === 'string')
    {
        stringMsg = msg + ' ';
    }
    else if (typeof msg === 'function')
    {
        stringMsg = '[function] ';
    }
    else if (msg && msg.stack && msg.message)
    {
        stringMsg = msg.message + ' ';
    }
    else
    {
        try
        {
            stringMsg = '\n' + JSON.stringify(msg, undefined, 2) + '\n';
        }
        catch (error)
        {
            stringMsg += 'Undefined Message';
        }
    }
    return stringMsg;
};
