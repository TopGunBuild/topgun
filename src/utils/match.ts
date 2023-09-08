import { isString, isObject, isDefined } from '@topgunbuild/typed';
import { LEX } from '../types';

export function match(template: string, options: LEX|string): boolean
{
    if (!isString(template))
    {
        return false;
    }
    else if (!isObject(options))
    {
        options = {};
    }

    if (isDefined(options['=']))
    {
        return template === options['='];
    }
    else if (isString(options['*']))
    {
        return template.startsWith(options['*']);
    }
    else if (isString(options['>']) && isString(options['<']))
    {
        return template >= options['>'] && template <= options['<'];
    }
    else if (isString(options['>']) && template >= options['>'])
    {
        return true;
    }
    else if (isString(options['<']) && template <= options['<'])
    {
        return true;
    }
    return false;
}
