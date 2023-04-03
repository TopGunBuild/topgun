import { isString } from './is-string';

export function getFirstLetter(value: string): string
{
    return isString(value) && value.slice(0, 1);
}
