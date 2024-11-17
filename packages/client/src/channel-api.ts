import { MessageRow, PutMessageAction, StoreItem, SelectResult, SelectAction, SelectOptions } from "@topgunbuild/models";
import { LoggerService } from "@topgunbuild/logger";
import { Store } from "./store";
import { QueryCb } from "./types";
import { StoreError } from "./errors";
import { bigintTime } from "@topgunbuild/time";

export class ChannelAPI {
    constructor(
        private readonly channelId: string,
        private readonly store: Store,
        private readonly logger: LoggerService
    ) {}

    /**
     * Subscribe to messages in the channel
     * @param options - The select options
     * @param cb - The callback
     * @returns A function to unsubscribe from the messages
     */
    public subscribeMessages(options: SelectOptions, cb: QueryCb<SelectResult<MessageRow>>): () => void {
        const query = new SelectAction({ entity: 'message', channelId: this.channelId, ...options });
        return this.store.subscribeQuery<MessageRow>(query, cb);
    }

    /**
     * Add messages to the channel
     * @param messages - The messages to add
     */
    public async addMessages<T extends StoreItem>(messages: T[]): Promise<void> {
        try {
            if (!Array.isArray(messages) || messages.length === 0) {
                throw new StoreError('Invalid messages array', 'INVALID_INPUT');
            }

            await this.store.upsert('message', messages);

            for (const message of messages) {
                const body = new PutMessageAction({
                    channelId: this.channelId,
                    messageId: message.$id,
                    value: JSON.stringify(message),
                    state: bigintTime()
                });
                await this.store.dispatchAction(body);
            }
        } catch (error) {
            this.logger.error('Failed to add messages:', error);
            throw error instanceof StoreError ? error : new StoreError('Failed to add messages', 'ADD_MESSAGE_ERROR');
        }
    }
}