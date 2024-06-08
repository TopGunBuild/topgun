import { isArray, isObject } from '@topgunbuild/utils';
import { Client } from '../client';
import { PutValue } from '../types';
import { Scope } from './scope';

export class NodeScope extends Scope
{
    constructor(client: Client, path: string)
    {
        super(client, path);
    }

    async put(values: PutValue): Promise<void>
    async put(values: PutValue[]): Promise<void>
    async put(values: PutValue[]|PutValue): Promise<void>
    {
        if (isArray(values))
        {
            await Promise.all(
                values.map(value => this.#put(value)),
            );
        }
        else
        {
            return this.#put(values);
        }
    }

    async #put(value: PutValue): Promise<void>
    {
        if (this._authRequired())
        {
            throw new Error(
                'You cannot save data to user space if the user is not authorized.',
            );
        }
        else if (!isObject(value))
        {
            throw new Error(
                'Data at root of graph must be an object.',
            );
        }

        const fieldNames = Object.keys(value);

        await Promise.all(
            fieldNames.map(field_name => this._putRecord(field_name, value[field_name]))
        );
    }
}
