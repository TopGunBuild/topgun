import { StoreConnector } from "@topgunbuild/control-flow";
import WebSocket from "isomorphic-ws";
import { ConnectorState } from "@topgunbuild/control-flow";
import { TransportPayloadImpl } from "@topgunbuild/models";

type InboundMsg = Uint8Array;
type ProcessedInbound = TransportPayloadImpl;
type OutboundMsg = Uint8Array;
type ProcessedOutbound = Uint8Array;

export class WebSocketConnector extends StoreConnector<InboundMsg, OutboundMsg, ProcessedInbound, ProcessedOutbound> {
    private ws: WebSocket | null = null;

    constructor(private readonly config: {
        websocketURI: string;
        appId: string;
    }) {
        super();

        this.outputQueue.on('completed', (data) => {
            this.ws.send(data);
        });
    }

    /**
     * Use input middleware
     * @param middleware The middleware function to add
     */
    useInputMiddleware(middleware: (a: InboundMsg) => Promise<ProcessedInbound>|ProcessedInbound|undefined): WebSocketConnector {
        this.inputQueue.middleware.use(middleware);
        return this;
    }

    /**
     * Use output middleware
     * @param middleware The middleware function to add
     */
    useOutputMiddleware(middleware: (a: OutboundMsg) => Promise<ProcessedOutbound>|ProcessedOutbound|undefined): WebSocketConnector {
        this.outputQueue.middleware.use(middleware);
        return this;
    }

     /**
     * Manually disconnects from the WebSocket server.
     */
    public disconnect(): void {
        if (this.ws) {
            this.ws.close();
        }
    }

    /**
     * Starts the WebSocket connection.
     */
    public startSocket(): void {
        this.emit('stateChange', 'connecting' as ConnectorState);
        this.ensurePreviousSocketClosed();
        this.ws = new WebSocket(`${this.config.websocketURI}?app_id=${this.config.appId}`);
        this.ws.binaryType = 'arraybuffer';

        this.ws.addEventListener("open", () => {
            this.onOpen();
        });
        this.ws.addEventListener("message", event => {
            this.ingest(event.data as Uint8Array);
        });
        this.ws.addEventListener("close", () => {
            this.onClose();
        });
        this.ws.addEventListener("error", event => {
            this.onError(event)
        });
    }

    /**
     * Ensures the previous WebSocket is closed.
     */
    private ensurePreviousSocketClosed(): void {
        if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) {
            this.ws.close();
        }
    }

     /**
     * Handles the WebSocket connection open event.
     */
     private onOpen(): void {
        this.emit('stateChange', 'opened' as ConnectorState);
     }
 
     /**
      * Handles the WebSocket connection close event.
      */
     private onClose(): void {
        this.emit('stateChange', 'closed' as ConnectorState);
     }
 
     /**
      * Handles the WebSocket connection error.
      * @param event - The error event.
      */
     private onError({ error }: WebSocket.ErrorEvent): void {
        this.emit('stateChange', 'errored' as ConnectorState);
     }
}
