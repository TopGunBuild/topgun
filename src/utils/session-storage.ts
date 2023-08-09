import { TGSupportedStorage } from '../types';
import { storageStruct } from './assert';
import { localStorageAdapter } from './local-storage';
import { TG_CLIENT_DEFAULT_OPTIONS } from '../client/client-options';

export function getSessionStorage(sessionStorage: TGSupportedStorage|undefined|boolean): TGSupportedStorage|null
{
    return !sessionStorage
        ? null
        : storageStruct(sessionStorage).ok
            ? (sessionStorage as TGSupportedStorage)
            : localStorageAdapter;
}

export function getSessionStorageKey(sessionStorageKey: string): string
{
    return sessionStorageKey || TG_CLIENT_DEFAULT_OPTIONS.sessionStorageKey
}