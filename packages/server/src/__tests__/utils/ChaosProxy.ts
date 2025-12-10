import { WebSocket, WebSocketServer, RawData } from 'ws';

export interface ChaosConfig {
  latencyMs: number;    // Fixed latency in ms
  jitterMs: number;     // Random extra latency in ms (0 to jitter)
  flakeRate: number;    // 0.0 to 1.0 probability of dropping a message
  isSilent: boolean;    // If true, connection stays open but messages are dropped (blackhole)
}

export class ChaosProxy {
  private wss: WebSocketServer;
  private activeConnections: Set<{ client: WebSocket; server: WebSocket }> = new Set();
  private _actualPort: number = 0;
  private _readyPromise: Promise<number>;
  private _readyResolve!: (port: number) => void;
  private _stopped: boolean = false;

  private config: ChaosConfig = {
    latencyMs: 0,
    jitterMs: 0,
    flakeRate: 0,
    isSilent: false
  };

  constructor(private readonly port: number, private readonly targetUrl: string) {
    this._readyPromise = new Promise((resolve) => {
      this._readyResolve = resolve;
    });

    this.wss = new WebSocketServer({ port });

    this.wss.on('listening', () => {
      const addr = this.wss.address();
      this._actualPort = typeof addr === 'object' && addr ? addr.port : port;
      this._readyResolve(this._actualPort);
    });

    this.wss.on('connection', (clientSocket) => this.handleConnection(clientSocket));
  }

  /** Wait for proxy to be ready and get actual port */
  public ready(): Promise<number> {
    return this._readyPromise;
  }

  /** Get the actual port the proxy is listening on */
  public get actualPort(): number {
    return this._actualPort;
  }

  public updateConfig(newConfig: Partial<ChaosConfig>) {
    this.config = { ...this.config, ...newConfig };
  }

  public disconnectAll() {
    for (const conn of this.activeConnections) {
      conn.client.terminate();
      conn.server.terminate();
    }
    this.activeConnections.clear();
  }

  public stop() {
    this._stopped = true;
    this.disconnectAll();
    this.wss.close();
  }

  private handleConnection(clientSocket: WebSocket) {
    if (this._stopped) return;

    const serverSocket = new WebSocket(this.targetUrl);
    const connection = { client: clientSocket, server: serverSocket };
    this.activeConnections.add(connection);

    // Buffer messages until server connection is open
    const clientBuffer: RawData[] = [];
    let isServerOpen = false;

    serverSocket.on('open', () => {
      isServerOpen = true;
      // Flush buffer
      while (clientBuffer.length > 0) {
        const data = clientBuffer.shift();
        if (data) this.forwardMessage(data, serverSocket);
      }
    });

    serverSocket.on('close', () => {
        if (!this._stopped) {
            clientSocket.close();
        }
        this.activeConnections.delete(connection);
    });

    serverSocket.on('error', () => {
        if (!this._stopped) {
            clientSocket.close();
        }
    });

    clientSocket.on('message', (data) => {
      if (!isServerOpen) {
        clientBuffer.push(data);
      } else {
        this.forwardMessage(data, serverSocket);
      }
    });

    serverSocket.on('message', (data) => {
      this.forwardMessage(data, clientSocket);
    });

    clientSocket.on('close', () => {
        if (!this._stopped) {
            serverSocket.close();
        }
        this.activeConnections.delete(connection);
    });

    clientSocket.on('error', () => {
        if (!this._stopped) {
            serverSocket.close();
        }
    });
  }

  private forwardMessage(data: RawData, target: WebSocket) {
    if (this.config.isSilent) {
        // Blackhole: do nothing
        return;
    }

    if (this.config.flakeRate > 0 && Math.random() < this.config.flakeRate) {
        return;
    }

    const delay = this.config.latencyMs + (Math.random() * this.config.jitterMs);

    if (delay > 0) {
        setTimeout(() => {
            if (target.readyState === WebSocket.OPEN) {
                target.send(data);
            }
        }, delay);
    } else {
        if (target.readyState === WebSocket.OPEN) {
            target.send(data);
        }
    }
  }
}
