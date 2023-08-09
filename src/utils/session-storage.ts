import { TGSupportedStorage } from '../types';
import { storageStruct } from './assert';
import { localStorageAdapter } from './local-storage';

export function getSessionStorage(sessionStorage: TGSupportedStorage|undefined|boolean): TGSupportedStorage|null
{
    return !sessionStorage
        ? null
        : storageStruct(sessionStorage).ok
            ? (sessionStorage as TGSupportedStorage)
            : localStorageAdapter;
}