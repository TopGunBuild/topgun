import { isSupport } from './is-support';
import { isString } from './is-string';

export function isValid(value: any): boolean
{
    return isSupport(value)
        || (!!value && isString(value['#']) && Object.keys(value).length === 1);
}