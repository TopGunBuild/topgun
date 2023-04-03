type IObject = {[key: string]: any};

export function isObject<T extends IObject>(value: any): value is T
{
    return value && value.toString() === '[object Object]';
}

