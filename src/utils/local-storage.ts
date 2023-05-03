import { isBrowser } from './is-browser';
import { TGSupportedStorage } from '../types';
import globalThis from '../utils/window-or-global';

export const localStorageAdapter: TGSupportedStorage = {
    getItem: (key) =>
    {
        if (!isBrowser())
        {
            return null;
        }

        return globalThis.localStorage.getItem(key);
    },
    setItem: (key, value) =>
    {
        if (!isBrowser())
        {
            return;
        }

        globalThis.localStorage.setItem(key, value);
    },
    removeItem: (key) =>
    {
        if (!isBrowser())
        {
            return;
        }

        globalThis.localStorage.removeItem(key);
    },
};
