import { Identifiable } from "./utils";

export interface ChannelInfo extends Identifiable {
    name: string;
    description?: string;
}
