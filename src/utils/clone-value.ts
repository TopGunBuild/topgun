import { isObject } from './is-object';

export const cloneValue = (value: any, deep = false): any => 
{
    if (Array.isArray(value)) 
{
        if (deep) 
{
            const arr: any[] = [];
            if (!value) 
{
                return arr;
            }
            let i = value.length;
            while (i--) 
{
                arr[i] = deep ? cloneValue(value[i]) : value[i];
            }
            return arr;
        }
 else 
{
            return [...value];
        }
    }

    if (isObject(value)) 
{
        let result = {};

        if (deep) 
{
            for (const key of Object.keys(value)) 
{
                result[key] = cloneValue(value[key]);
            }
        }
 else 
{
            result = { ...value };
        }
        return result;
    }
    return value;
};
