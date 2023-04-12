import { TGSupportedStorage } from '../types';

export const setItemAsync = async (
    storage: TGSupportedStorage,
    key: string,
    data: any,
): Promise<void> => 
{
    await storage.setItem(key, JSON.stringify(data));
};

export const getItemAsync = async (
    storage: TGSupportedStorage,
    key: string,
): Promise<unknown> => 
{
    const value = await storage.getItem(key);

    if (!value) 
{
        return null;
    }

    try 
{
        return JSON.parse(value);
    }
 catch 
{
        return value;
    }
};

export const removeItemAsync = async (
    storage: TGSupportedStorage,
    key: string,
): Promise<void> => 
{
    await storage.removeItem(key);
};
