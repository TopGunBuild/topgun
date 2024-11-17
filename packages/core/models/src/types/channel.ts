import { Identifiable } from "./utils";

export interface Channel extends Identifiable {
    name: string;
    description?: string;
}
