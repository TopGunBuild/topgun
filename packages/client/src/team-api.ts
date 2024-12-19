import { Store } from "./store";
import { 
    LocalUserContext,
    Member,
    PermissionsMap,
    Team,
    KeysetPrivateInfo,
    TeamInfo,
} from "@topgunbuild/models";
import { LoggerService } from "@topgunbuild/logger";
import { EventEmitter } from "@topgunbuild/eventemitter";
import { ChannelAPI } from "./channel-api";
import { TeamReader } from "./team-reader";
import { TeamWriter } from "./team-writer";

export class TeamAPI extends EventEmitter {
    private readonly reader: TeamReader;
    private readonly writer: TeamWriter;

    constructor(
        private readonly team: TeamInfo,
        private readonly store: Store,
        teamKeys: KeysetPrivateInfo,
        private readonly logger: LoggerService,
        private readonly context: LocalUserContext,
        private readonly seed: string
    ) {
        super();
        this.reader = new TeamReader(team, store, logger, context);
        this.writer = new TeamWriter(team, store, logger, teamKeys, this.reader, seed);
    }

    // Delegate read operations to TeamReader
    public get id() { return this.reader.id; }
    public get name() { return this.reader.name; }
    public hasMember(userId: string) { return this.reader.hasMember(userId); }
    public getMember(userId: string) { return this.reader.getMember(userId); }
    public memberHasRole(userId: string, roleName: string) { return this.reader.memberHasRole(userId, roleName); }
    public memberIsAdmin(userId: string) { return this.reader.memberIsAdmin(userId); }
    public hasRole(roleName: string) { return this.reader.hasRole(roleName); }
    public getTeamKeys(generation?: number) { return this.reader.getTeamKeys(generation); }
    public getRoleKeys(roleName: string, generation?: number) { return this.reader.getRoleKeys(roleName, generation); }
    public getAdminKeys(generation?: number) { return this.reader.getAdminKeys(generation); }
    
    // Delegate write operations to TeamWriter
    public updateTeam(params: {name?: string, description?: string}) { return this.writer.updateTeam(params); }
    public addMember(member: Member, roles?: string[]) { return this.writer.addMember(member, roles); }
    public removeMember(userId: string) { return this.writer.removeMember(userId); }
    public addRole(roleName: string, permissions?: PermissionsMap) { return this.writer.addRole(roleName, permissions); }
    public removeRole(roleName: string) { return this.writer.removeRole(roleName); }
    public assignRoleToMember(userId: string, roleName: string) { return this.writer.assignRoleToMember(userId, roleName); }
    public removeRoleFromMember(userId: string, roleName: string) { return this.writer.removeRoleFromMember(userId, roleName); }
    public changeKeys(newKeys: KeysetPrivateInfo) { return this.writer.changeKeys(newKeys); }
    public inviteMember(params: {
        seed?: string,
        expiration?: number,
        maxUses?: number
    }) { return this.writer.inviteMember(params); }

    /**
     * Get a channel API for a specific channel
     */
    public channel(channelId: string): ChannelAPI {
        return new ChannelAPI(channelId, this.store, this.logger);
    }
}