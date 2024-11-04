import { MessageRow, SelectOptions, SelectResult } from "@topgunbuild/types";
import { Store } from "./store";
import { QueryCb } from "./types";
import { SelectRequest } from "@topgunbuild/transport";

export class TeamService
{
    readonly teamId: string;
    readonly store: Store;

    constructor(teamId: string, store: Store)
    {
        this.teamId = teamId;
        this.store = store;
    }

    subscribeMessages(options: SelectOptions, cb: QueryCb<SelectResult<MessageRow>>): () => void
    {
        const query = new SelectRequest({ entity: 'message', ...options });
        return this.store.subscribeQuery<MessageRow>(query, cb);
    }
}
