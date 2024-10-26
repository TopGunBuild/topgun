import WebSocket from "isomorphic-ws";

export enum WebSocketReadyState {
    CONNECTING = 0,
    OPEN = 1,
    CLOSING = 2,
    CLOSED = 3,
}

// export const WebSocketStatus = {
//     CONNECTING: "connecting",
//     OPENED: "opened",
//     AUTHENTICATED: "authenticated",
//     CLOSED: "closed",
//     ERRORED: "errored",
// };

export type MessageType = string | ArrayBuffer | Blob | Uint8Array;

export interface QueuedMessage {
    data: WebSocket.Data;
    resolve: () => void;
    reject: (reason?: any) => void;
}

export interface WebSocketManagerConfig {
    websocketURI: string;
    appId: string;
}
