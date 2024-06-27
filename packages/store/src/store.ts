import { CloseIteratorQuery, SelectNextQuery, SelectQuery } from '@topgunbuild/transport';
import { StoreValue } from './store-value';
import { PublicKey } from '@topgunbuild/crypto';
import { StoreResults } from './result';
import { IdKey } from './id';

export interface Store
{
    start(): Promise<void>|void;

    stop(): Promise<void>|void;

    get(id: IdKey): Promise<StoreValue|undefined>;

    put(value: StoreValue): Promise<void>|void;

    del(id: IdKey): Promise<void>|void;

    select(query: SelectQuery, from: PublicKey): Promise<StoreResults>;

    getSize(): number|Promise<number>;

    next(query: SelectNextQuery, from: PublicKey): Promise<StoreResults>;

    close(query: CloseIteratorQuery, from: PublicKey): Promise<void>|void;

    iterator(): IterableIterator<[string, StoreValue]>;

    get cursorCount(): number;
}
