import { bigintTime } from '@topgunbuild/time';
import {
    Message,
    MessageHeader,
    PutMessage, Query, SelectOptions,
    Sort,
} from '@topgunbuild/transport';
import { DataNode, DataValue } from '@topgunbuild/store';
import { isEmptyObject, isObject, toArray } from '@topgunbuild/utils';
import { v4 as uuidv4 } from 'uuid';
import { ClientProviders } from './client-providers';

export class ClientQueryBuilder
{
    readonly #section: string;
    readonly #providers: ClientProviders;

    constructor(path: string, providers: ClientProviders)
    {
        this.#section   = path;
        this.#providers = providers;
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Public methods
    // -----------------------------------------------------------------------------------------------------

    async select(
        options?: SelectOptions&{
            fields?: string[];
            query?: Query[];
            sort?: Sort[];
        },
    )
    {
        // Set fetch based on either the size or the maximum value (default to maximum u32 (4294967295))
        // queryMessage.fetch = queryMessage.fetch ?? 0xffffffff;
    }

    async insert(values: DataNode): Promise<void>
    async insert(values: DataNode[]): Promise<void>
    async insert(values: DataNode[]|DataNode): Promise<void>
    {
        await Promise.all(
            toArray(values).map(value => this.#putNode(uuidv4(), value)),
        );
    }

    node(nodeId: string)
    {
        return {
            select: async (
                options?: SelectOptions&{
                    fields?: string[];
                },
            ) =>
            {

            },
            put   : async (node: DataNode) =>
            {
                return this.#putNode(nodeId, node);
            },
            delete: async () =>
            {

            },
            field : (field: string) =>
            {
                return {
                    select: async () =>
                    {

                    },
                    put   : async (value: DataValue) =>
                    {
                        return this.#putValue(nodeId, field, value);
                    },
                    delete: async () =>
                    {

                    },
                };
            },
        };
    }

    // -----------------------------------------------------------------------------------------------------
    // @ Private methods
    // -----------------------------------------------------------------------------------------------------

    async #putNode(node_id: string, value: DataNode): Promise<void>
    {
        if (this.#authRequired())
        {
            throw new Error(
                'You cannot save data to user space if the user is not authorized.',
            );
        }
        else if (!isObject(value))
        {
            throw new Error('Node must be an object.');
        }
        else if (isEmptyObject(value))
        {
            throw new Error('Node must not be an empty object.');
        }
        else if (!this.#providers.store)
        {
            await this.#providers.waitForStoreInit();
        }

        await Promise.all(
            Object.keys(value).map(field => this.#putValue(node_id, field, value[field])),
        );
    }

    async #putValue(node_id: string, field: string, value: DataValue): Promise<any>
    {
        const message = new Message({
            header: new MessageHeader({}),
            data  : new PutMessage(
                this.#section,
                node_id,
                field,
                bigintTime(),
                value,
            ).encode(),
        });
    }

    #authRequired(): boolean
    {
        return false;
    }
}
