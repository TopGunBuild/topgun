import { isObject } from './is-object';

/**
 * https://mathiasbynens.be/notes/globalthis
 */
export function polyfillGlobalThis(): void
{
    if (isObject(globalThis)) return;
    try
    {
        Object.defineProperty(Object.prototype, '__magic__', {
            get         : function ()
            {
                return this
            },
            configurable: true,
        });
        // @ts-ignore
        __magic__.globalThis = __magic__;
        // @ts-ignore
        delete Object.prototype.__magic__
    }
    catch (e)
    {
        if (typeof self !== 'undefined')
        {
            // @ts-ignore
            self.globalThis = self
        }
    }
}
