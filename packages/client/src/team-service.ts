import { ISelectResult, MessageRow } from "@topgunbuild/types";
import { Store } from "./store";
import { QueryCb } from "./types";

export class TeamService
{
    readonly teamId: string;
    readonly store: Store;

    constructor(teamId: string, store: Store)
    {
        this.teamId = teamId;
        this.store = store;
    }

    subscribeMessages(options?: SelectSectionOptions, cb: QueryCb<ISelectResult<MessageRow>>): () => void
    {
        return this.store.subscribeQuery<MessageRow>(new SelectQuery(options), cb);
    }
}
