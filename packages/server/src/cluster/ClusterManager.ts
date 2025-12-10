import { WebSocket, WebSocketServer, ClientOptions as WsClientOptions } from 'ws';
import { EventEmitter } from 'events';
import * as dns from 'dns';
import { logger } from '../utils/logger';
import { readFileSync } from 'fs';
import * as https from 'https';
import { ClusterTLSConfig } from '../types/TLSConfig';

export interface ClusterConfig {
  nodeId: string;
  host: string;
  port: number;
  peers: string[]; // List of "host:port"
  discovery?: 'manual' | 'kubernetes';
  serviceName?: string;
  discoveryInterval?: number;
  tls?: ClusterTLSConfig;
}

export interface ClusterMember {
  nodeId: string;
  host: string;
  port: number;
  socket: WebSocket;
  isSelf: boolean;
}

export interface ClusterMessage {
  type: 'HELLO' | 'OP_FORWARD' | 'PARTITION_UPDATE' | 'HEARTBEAT' | 'CLUSTER_EVENT' | 'CLUSTER_QUERY_EXEC' | 'CLUSTER_QUERY_RESP' | 'CLUSTER_GC_REPORT' | 'CLUSTER_GC_COMMIT' | 'CLUSTER_LOCK_REQ' | 'CLUSTER_LOCK_RELEASE' | 'CLUSTER_LOCK_GRANTED' | 'CLUSTER_LOCK_RELEASED' | 'CLUSTER_CLIENT_DISCONNECTED' | 'CLUSTER_TOPIC_PUB';
  senderId: string;
  payload: any;
}

export class ClusterManager extends EventEmitter {
  public readonly config: ClusterConfig;
  private server?: WebSocketServer;
  private members: Map<string, ClusterMember> = new Map();
  private pendingConnections: Set<string> = new Set();
  private reconnectIntervals: Map<string, NodeJS.Timeout> = new Map();
  private discoveryTimer?: NodeJS.Timeout;

  constructor(config: ClusterConfig) {
    super();
    this.config = config;
  }

  private _actualPort: number = 0;

  /** Get the actual port the cluster is listening on */
  public get port(): number {
    return this._actualPort;
  }

  public start(): Promise<number> {
    return new Promise((resolve) => {
      logger.info({ port: this.config.port, tls: !!this.config.tls?.enabled }, 'Starting Cluster Manager');

      if (this.config.tls?.enabled) {
        // HTTPS-based WebSocket Server for cluster
        const tlsOptions = this.buildClusterTLSOptions();
        const httpsServer = https.createServer(tlsOptions);
        this.server = new WebSocketServer({ server: httpsServer });

        httpsServer.listen(this.config.port, () => {
          const addr = httpsServer.address();
          this._actualPort = typeof addr === 'object' && addr ? addr.port : this.config.port;
          logger.info({ port: this._actualPort }, 'Cluster Manager listening (TLS enabled)');
          this.onServerReady(resolve);
        });
      } else {
        this.server = new WebSocketServer({ port: this.config.port });

        this.server.on('listening', () => {
          const addr = this.server!.address();
          this._actualPort = typeof addr === 'object' && addr ? addr.port : this.config.port;
          logger.info({ port: this._actualPort }, 'Cluster Manager listening');
          this.onServerReady(resolve);
        });
      }

      this.server?.on('connection', (ws, req) => {
        logger.info({ remoteAddress: req.socket.remoteAddress }, 'Incoming cluster connection');
        this.handleSocket(ws, false);
      });
    });
  }

  /** Called when server is ready - registers self and initiates peer connections */
  private onServerReady(resolve: (port: number) => void): void {
    // Add self to members with actual port
    this.members.set(this.config.nodeId, {
      nodeId: this.config.nodeId,
      host: this.config.host,
      port: this._actualPort,
      socket: null as any,
      isSelf: true
    });

    // Connect to peers after we know our port
    if (this.config.discovery === 'kubernetes' && this.config.serviceName) {
      this.startDiscovery();
    } else {
      this.connectToPeers();
    }

    resolve(this._actualPort);
  }

  public stop() {
    logger.info({ port: this.config.port }, 'Stopping Cluster Manager');

    // Clear reconnect intervals
    for (const timeout of this.reconnectIntervals.values()) {
      clearTimeout(timeout);
    }
    this.reconnectIntervals.clear();
    if (this.discoveryTimer) {
      clearInterval(this.discoveryTimer);
      this.discoveryTimer = undefined;
    }
    this.pendingConnections.clear();

    // Close all peer connections
    for (const member of this.members.values()) {
      if (member.socket) {
        member.socket.terminate(); // Force close
      }
    }
    this.members.clear();

    // Close server
    if (this.server) {
      this.server.close();
    }
  }

  private connectToPeers() {
    for (const peer of this.config.peers) {
      this.connectToPeer(peer);
    }
  }

  private startDiscovery() {
    const runDiscovery = async () => {
      if (!this.config.serviceName) return;

      try {
        const addresses = await dns.promises.resolve4(this.config.serviceName);
        logger.debug({ addresses, serviceName: this.config.serviceName }, 'DNS discovery results');

        for (const ip of addresses) {
          // Use actual port if available (likely matching K8s config), fallback to config port
          const targetPort = this._actualPort || this.config.port;
          const peerAddress = `${ip}:${targetPort}`;
          // Attempt to connect. connectToPeer handles dupes and self-checks (via handshake eventually)
          this.connectToPeer(peerAddress);
        }
      } catch (err: any) {
        logger.error({ err: err.message, serviceName: this.config.serviceName }, 'DNS discovery failed');
      }
    };

    logger.info({ serviceName: this.config.serviceName }, 'Starting Kubernetes DNS discovery');
    runDiscovery();
    // Default to 10s if not specified, to be less aggressive
    this.discoveryTimer = setInterval(runDiscovery, this.config.discoveryInterval || 10000);
  }

  private scheduleReconnect(peerAddress: string, attempt: number = 0) {
    if (this.reconnectIntervals.has(peerAddress)) return;

    // Exponential backoff: 5s, 10s, 20s, 40s, 60s (max)
    const delay = Math.min(5000 * Math.pow(2, attempt), 60000);

    const timeout = setTimeout(() => {
      this.reconnectIntervals.delete(peerAddress);
      // Pass next attempt number
      this.connectToPeerWithBackoff(peerAddress, attempt + 1);
    }, delay);

    this.reconnectIntervals.set(peerAddress, timeout);
  }

  // Helper to track attempts
  private connectToPeerWithBackoff(peerAddress: string, attempt: number) {
    // We need to modify connectToPeer to accept attempt or create a wrapper.
    // To keep it simple without changing signature of connectToPeer everywhere,
    // we'll just call connectToPeer and let it fail -> scheduleReconnect -> increment attempt.
    // But connectToPeer logic needs to pass the attempt to scheduleReconnect on failure.
    // Refactoring connectToPeer to take optional attempt param.
    this._connectToPeerInternal(peerAddress, attempt);
  }

  private connectToPeer(peerAddress: string) {
    this._connectToPeerInternal(peerAddress, 0);
  }

  private _connectToPeerInternal(peerAddress: string, attempt: number) {
    if (this.pendingConnections.has(peerAddress)) return;

    // Check if already connected
    for (const member of this.members.values()) {
      if (`${member.host}:${member.port}` === peerAddress) return;
    }

    // PREVENT LOOP: ... (omitted comments)

    logger.info({ peerAddress, attempt, tls: !!this.config.tls?.enabled }, 'Connecting to peer');
    this.pendingConnections.add(peerAddress);

    try {
      let ws: WebSocket;

      if (this.config.tls?.enabled) {
        // Secure WebSocket connection
        const protocol = 'wss://';
        const wsOptions: WsClientOptions = {
          rejectUnauthorized: this.config.tls.rejectUnauthorized !== false,
        };

        // mTLS: Provide client certificate
        if (this.config.tls.certPath && this.config.tls.keyPath) {
          wsOptions.cert = readFileSync(this.config.tls.certPath);
          wsOptions.key = readFileSync(this.config.tls.keyPath);

          if (this.config.tls.passphrase) {
            wsOptions.passphrase = this.config.tls.passphrase;
          }
        }

        // CA for peer verification
        if (this.config.tls.caCertPath) {
          wsOptions.ca = readFileSync(this.config.tls.caCertPath);
        }

        ws = new WebSocket(`${protocol}${peerAddress}`, wsOptions);
      } else {
        // Plain WebSocket (development)
        ws = new WebSocket(`ws://${peerAddress}`);
      }

      ws.on('open', () => {
        this.pendingConnections.delete(peerAddress);
        logger.info({ peerAddress }, 'Connected to peer');
        // Reset backoff on success
        this.handleSocket(ws, true, peerAddress);
      });

      ws.on('error', (err) => {
        logger.error({ peerAddress, err: err.message }, 'Connection error to peer');
        this.pendingConnections.delete(peerAddress);
        this.scheduleReconnect(peerAddress, attempt);
      });

      ws.on('close', () => {
        this.pendingConnections.delete(peerAddress);
      });

    } catch (e) {
      this.pendingConnections.delete(peerAddress);
      this.scheduleReconnect(peerAddress, attempt);
    }
  }

  private handleSocket(ws: WebSocket, initiated: boolean, peerAddress?: string) {
    // Handshake: Send my NodeID with actual port (not config port which may be 0)
    const helloMsg: ClusterMessage = {
      type: 'HELLO',
      senderId: this.config.nodeId,
      payload: {
        host: this.config.host,
        port: this._actualPort || this.config.port
      }
    };
    ws.send(JSON.stringify(helloMsg));

    let remoteNodeId: string | null = null;

    ws.on('message', (data) => {
      try {
        const msg = JSON.parse(data.toString()) as ClusterMessage;

        if (msg.type === 'HELLO') {
          remoteNodeId = msg.senderId;
          const { host, port } = msg.payload;
          logger.info({ nodeId: remoteNodeId, host, port }, 'Peer identified');

          // Tie-Breaker Rule: Connection initiated by Low ID wins.
          // Initiator < Receiver = Valid.
          // Initiator > Receiver = Invalid (Drop).

          const myId = this.config.nodeId;
          const otherId = remoteNodeId;

          // Determine who initiated this specific socket
          const initiatorId = initiated ? myId : otherId;
          const receiverId = initiated ? otherId : myId;

          /*
          // Tie-Breaker Rule: Connection initiated by Low ID wins.
          // Initiator < Receiver = Valid.
          // Initiator > Receiver = Invalid (Drop).
          // 
          // DISABLED: This strict rule prevents High-ID nodes from joining Low-ID seeds (common pattern).
          // We only use this for duplicate resolution now.
          
          if (initiatorId >= receiverId) {
              logger.info({ initiatorId, receiverId }, 'Dropping connection (Low-ID Initiator Policy)');
              try {
                  ws.close();
              } catch(e) {}
              return;
          }
          */

          // If we get here, this is a VALID connection.
          // Check if we somehow already have a connection
          if (this.members.has(remoteNodeId)) {
            logger.warn({ nodeId: remoteNodeId }, 'Duplicate valid connection. Replacing.');
            // In a real production system, we should use the Tie-Breaker here to decide which one to keep
            // to avoid split-brain socket usage.
            // For now, 'Replacing' means Last-Write-Wins on the connection slot.
          }

          this.members.set(remoteNodeId, {
            nodeId: remoteNodeId,
            host,
            port,
            socket: ws,
            isSelf: false
          });

          this.emit('memberJoined', remoteNodeId);
        } else {
          this.emit('message', msg);
        }
      } catch (err) {
        logger.error({ err }, 'Failed to parse cluster message');
      }
    });

    ws.on('close', () => {
      if (remoteNodeId) {
        // Only handle disconnect if this was the ACTIVE socket
        // This prevents "duplicate connection" cleanup from killing the valid session
        const current = this.members.get(remoteNodeId);
        if (current && current.socket === ws) {
          logger.info({ nodeId: remoteNodeId }, 'Peer disconnected');
          this.members.delete(remoteNodeId);
          this.emit('memberLeft', remoteNodeId);

          // If we initiated, we should try to reconnect
          if (initiated && peerAddress) {
            // Start with 0 attempt on fresh disconnect? 
            // Or maybe we should consider this a failure and backoff?
            // Let's restart with 0 for now as it might be a temp network blip
            this.scheduleReconnect(peerAddress, 0);
          }
        } else {
          // console.log(`Ignored close from stale/duplicate socket for ${remoteNodeId}`);
        }
      }
    });
  }

  public send(nodeId: string, type: ClusterMessage['type'], payload: any) {
    const member = this.members.get(nodeId);
    if (member && member.socket && member.socket.readyState === WebSocket.OPEN) {
      const msg: ClusterMessage = {
        type,
        senderId: this.config.nodeId,
        payload
      };
      member.socket.send(JSON.stringify(msg));
    } else {
      logger.warn({ nodeId }, 'Cannot send to node: not connected');
    }
  }

  public sendToNode(nodeId: string, message: any) {
    this.send(nodeId, 'OP_FORWARD', message);
  }

  public getMembers(): string[] {
    return Array.from(this.members.keys());
  }

  public isLocal(nodeId: string): boolean {
    return nodeId === this.config.nodeId;
  }

  private buildClusterTLSOptions(): https.ServerOptions {
    const config = this.config.tls!;

    const options: https.ServerOptions = {
      cert: readFileSync(config.certPath),
      key: readFileSync(config.keyPath),
      minVersion: config.minVersion || 'TLSv1.2',
    };

    if (config.caCertPath) {
      options.ca = readFileSync(config.caCertPath);
    }

    if (config.requireClientCert) {
      options.requestCert = true;
      options.rejectUnauthorized = true;
    }

    if (config.passphrase) {
      options.passphrase = config.passphrase;
    }

    return options;
  }
}

