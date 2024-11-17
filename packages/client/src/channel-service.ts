import { EventEmitter } from "@topgunbuild/eventemitter";
import { TeamService } from "./team-service";
import { Store } from "./store";
import { ConsoleLogger, LoggerService } from "@topgunbuild/logger";
import { SelectResult } from "@topgunbuild/models";
import { SelectOptions, SelectAction, MessageRow } from "@topgunbuild/models";
import { QueryCb } from "./types";

export class ChannelService extends EventEmitter {
    private readonly store: Store;
    private readonly teamService: TeamService;
    private readonly logger: LoggerService;
    private readonly channelId: string;

    constructor(
        channelId: string,
        store: Store,
        teamService: TeamService
    ) {
        super();
        this.channelId = channelId;
        this.store = store;
        this.teamService = teamService;
        this.logger = new ConsoleLogger('ChannelService');
    }

    public subscribeMessages(options: SelectOptions, cb: QueryCb<SelectResult<MessageRow>>): () => void {
        const query = new SelectAction({ entity: 'message', channelId: this.channelId, ...options });
        return this.store.subscribeQuery<MessageRow>(query, cb);
    }
}