import { SupportedStorage } from '../types';

export const setItemAsync = async (
    storage: SupportedStorage,
    key: string,
    data: any
): Promise<void> =>
{
    await storage.setItem(key, JSON.stringify(data))
};

export const getItemAsync = async (storage: SupportedStorage, key: string): Promise<unknown> =>
{
    const value = await storage.getItem(key);

    if (!value)
    {
        return null
    }

    try
    {
        return JSON.parse(value)
    }
    catch
    {
        return value
    }
};

export const removeItemAsync = async (storage: SupportedStorage, key: string): Promise<void> =>
{
    await storage.removeItem(key)
};
