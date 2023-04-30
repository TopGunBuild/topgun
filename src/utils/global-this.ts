import { isObject } from 'topgun-typed';

declare const __magic__: any;

/**
 * https://mathiasbynens.be/notes/globalthis
 */
export function polyfillGlobalThis(): void 
{
    if (isObject(globalThis)) return;
    try 
    {
        Object.defineProperty(Object.prototype, '__magic__', {
            get: function () 
            {
                return this;
            },
            configurable: true,
        });
        __magic__.globalThis = __magic__;
        delete Object.prototype['__magic__'];
    }
    catch (e) 
    {
        if (typeof self !== 'undefined') 
        {
            (self as any).globalThis = self;
        }
    }
}
