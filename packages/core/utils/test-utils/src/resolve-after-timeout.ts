import { wait } from './wait';

export async function resolveAfterTimeout<T extends any>(duration: number, value: T): Promise<T> {
    await wait(duration);
    return value;
}
