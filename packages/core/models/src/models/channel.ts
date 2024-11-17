import { EncodeHelper } from "../utils/encode-helper";
import { Channel } from "../types";
import { field, option } from "@dao-xyz/borsh";
import { randomId } from "@topgunbuild/common";

export class ChannelImpl extends EncodeHelper implements Channel {
    @field({ type: 'string' })
    $id: string;

    @field({ type: 'string' })
    name: string;

    @field({ type: option('string') })
    description?: string;

    constructor(data: {
        $id?: string,
        name: string,
        description?: string
    }) {
        super();
        this.$id = data.$id || randomId(32);
        this.name = data.name;
        this.description = data.description;
    }
}   
