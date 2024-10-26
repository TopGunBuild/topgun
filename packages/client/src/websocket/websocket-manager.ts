import { Deferred } from "@topgunbuild/utils";
import { ConsoleLogger, LoggerService } from "@topgunbuild/logger";
import WebSocket from "isomorphic-ws"
import { MessageType, QueuedMessage, WebSocketManagerConfig, WebSocketReadyState } from "./types";

/**
 * Manages the WebSocket connection and message sending.
 */
export class WebSocketManager {
    private config: WebSocketManagerConfig;
    private ws: WebSocket | null = null;
    private isManualClose = false;
    private reconnectTimeoutMs = 0;
    private reconnectTimeoutId: NodeJS.Timeout | null = null;
    private messageHandlers: ((data: WebSocket.Data) => void)[] = [];
    private connectionPromise: Deferred<void> | null = null;
    private messageQueue: QueuedMessage[] = [];
    private log: LoggerService;

    /**
     * Constructs a new WebSocketManager.
     * @param config - The WebSocketManager configuration.
     */
    constructor(config: WebSocketManagerConfig) {
        this.config = config;
        this.log = new ConsoleLogger("WebSocketManager");
    }

    /**
     * Connects to the WebSocket server.
     * @returns A promise that resolves when the connection is established.
     */
    public connect(): Promise<void> {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            return Promise.resolve();
        }

        if (this.connectionPromise) {
            return this.connectionPromise.promise;
        }

        this.connectionPromise = new Deferred<void>();
        this.startSocket();

        return this.connectionPromise.promise;
    }

    /**
     * Manually disconnects from the WebSocket server.
     */
    public disconnect(): void {
        this.isManualClose = true;
        if (this.ws) {
            this.ws.close();
        }
        if (this.reconnectTimeoutId) {
            clearTimeout(this.reconnectTimeoutId);
        }
        this.messageQueue = [];
    }

    /**
     * Sends a message to the WebSocket server.
     * @param data - The message to send.
     * @returns A promise that resolves when the message is sent.
     */
    public send(data: WebSocket.Data): Promise<void> {
        return new Promise((resolve, reject) => {
            if (this.ws && this.ws.readyState === WebSocketReadyState.OPEN) {
                this.sendImmediate(data).then(resolve).catch(reject);
            } else {
                this.messageQueue.push({ data, resolve, reject });
            }
        });
    }

    /**
     * Adds a message handler.
     * @param handler - The handler to add.
     */
    public addMessageHandler(handler: (data: WebSocket.Data) => void): void {
        this.messageHandlers.push(handler);
    }

    /**
     * Removes a message handler.
     * @param handler - The handler to remove.
     */
    public removeMessageHandler(handler: (data: WebSocket.Data) => void): void {
        this.messageHandlers = this.messageHandlers.filter(h => h !== handler);
    }

    /**
     * Starts the WebSocket connection.
     */
    public startSocket(): void {
        this.ensurePreviousSocketClosed();
        this.ws = new WebSocket(`${this.config.websocketURI}?app_id=${this.config.appId}`);
        this.ws.binaryType = 'arraybuffer';
        // this.ws.onopen = this.onOpen.bind(this);
        // this.ws.onmessage = this.onMessage.bind(this);
        // this.ws.onclose = this.onClose.bind(this);
        // this.ws.onerror = this.onError.bind(this);

        this.ws.addEventListener("open", () => {
            this.onOpen()
        });
        this.ws.addEventListener("message", event => {
            this.onMessage(event)
        });
        this.ws.addEventListener("close", () => {
            this.onClose()
        });
        this.ws.addEventListener("error", event => {
            this.onError(event)
        });
    }

    /**
     * Flushes the message queue.
     */
    private flushQueue(): void {
        while (this.messageQueue.length > 0) {
            const message = this.messageQueue.shift();
            if (message) {
                this.sendImmediate(message.data)
                    .then(message.resolve)
                    .catch(message.reject);
            }
        }
    }

    /**
     * Ensures the previous WebSocket is closed.
     */
    private ensurePreviousSocketClosed(): void {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.isManualClose = true;
            this.ws.close();
        }
    }

    /**
     * Handles the WebSocket connection open event.
     */
    private onOpen(): void {
        this.log.log("[socket] connected");
        this.reconnectTimeoutMs = 0;
        if (this.connectionPromise) {
            this.connectionPromise.resolve();
            this.connectionPromise = null;
        }
        this.flushQueue();
    }

    /**
     * Handles the WebSocket message event.
     * @param event - The message event.
     */
    private onMessage({ data }: WebSocket.MessageEvent): void {
        this.messageHandlers.forEach(handler => handler(data));
    }

    /**
     * Handles the WebSocket connection close event.
     */
    private onClose(): void {
        if (this.isManualClose) {
            this.isManualClose = false;
            this.log.log("[socket-close] manual close, will not reconnect");
            return;
        }

        this.log.log("[socket-close] scheduling reconnect", this.reconnectTimeoutMs);
        this.reconnectTimeoutId = setTimeout(() => {
            this.reconnectTimeoutMs = Math.min(this.reconnectTimeoutMs + 1000, 10000);
            this.startSocket();
        }, this.reconnectTimeoutMs);
    }

    /**
     * Handles the WebSocket connection error.
     * @param event - The error event.
     */
    private onError({ error }: WebSocket.ErrorEvent): void {
        this.log.error("[socket] error: ", error);
    }

    /**
     * Sends a message immediately if the WebSocket is open.
     * @param data - The message to send.
     * @returns A promise that resolves when the message is sent.
     */
    private sendImmediate(data: WebSocket.Data): Promise<void> {
        return new Promise((resolve, reject) => {
            if (this.ws && this.ws.readyState === WebSocketReadyState.OPEN) {
                this.ws.send(data);
                resolve();
            } else {
                reject(new Error('WebSocket is not open'));
            }
        });
    }
}