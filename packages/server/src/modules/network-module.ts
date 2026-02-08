import { createServer as createHttpServer, Server as HttpServer } from 'node:http';
import { createServer as createHttpsServer, Server as HttpsServer, ServerOptions as HttpsServerOptions } from 'node:https';
import { readFileSync } from 'node:fs';
import type { Socket } from 'node:net';
import { WebSocketServer } from 'ws';
import { logger } from '../utils/logger';
import { ConnectionRateLimiter } from '../utils/ConnectionRateLimiter';
import { RateLimitedLogger } from '../utils/RateLimitedLogger';
import type { NetworkModuleConfig, NetworkModuleDeps, NetworkModule } from './types';

// Helper to build TLS options
function buildTLSOptions(config: NonNullable<NetworkModuleConfig['tls']>): HttpsServerOptions {
  const options: HttpsServerOptions = {
    cert: readFileSync(config.certPath),
    key: readFileSync(config.keyPath),
    minVersion: config.minVersion || 'TLSv1.2',
  };
  if (config.caCertPath) options.ca = readFileSync(config.caCertPath);
  if (config.ciphers) options.ciphers = config.ciphers;
  if (config.passphrase) options.passphrase = config.passphrase;
  return options;
}

export function createNetworkModule(
  config: NetworkModuleConfig,
  _deps: NetworkModuleDeps
): NetworkModule {
  // Mutable request handler reference for deferred wiring.
  // The default handler serves a simple status page; it can be replaced
  // after assembly via setHttpRequestHandler() to add /sync routing.
  let currentRequestHandler = (_req: any, res: any) => {
    res.writeHead(200);
    res.end(config.tls?.enabled ? 'TopGun Server Running (Secure)' : 'TopGun Server Running');
  };

  // Dispatcher delegates to the mutable handler so the HTTP server can
  // be created once at construction time but route configuration can
  // change later via deferred wiring.
  const requestDispatcher = (req: any, res: any) => {
    currentRequestHandler(req, res);
  };

  // Create HTTP server (NOT listening yet)
  let httpServer: HttpServer | HttpsServer;
  if (config.tls?.enabled) {
    httpServer = createHttpsServer(buildTLSOptions(config.tls), requestDispatcher);
  } else {
    httpServer = createHttpServer(requestDispatcher);
  }

  // Configure server limits
  httpServer.maxConnections = config.maxConnections ?? 10000;
  httpServer.timeout = config.serverTimeout ?? 120000;
  httpServer.keepAliveTimeout = config.keepAliveTimeout ?? 5000;
  httpServer.headersTimeout = config.headersTimeout ?? 60000;

  // Configure socket-level options
  const socketNoDelay = config.socketNoDelay ?? true;
  const socketKeepAlive = config.socketKeepAlive ?? true;
  const socketKeepAliveMs = config.socketKeepAliveMs ?? 60000;

  httpServer.on('connection', (socket: Socket) => {
    socket.setNoDelay(socketNoDelay);
    socket.setKeepAlive(socketKeepAlive, socketKeepAliveMs);
  });

  // Create WebSocket server (attached to httpServer, NOT listening)
  const wss = new WebSocketServer({
    server: httpServer,
    backlog: config.wsBacklog ?? 511,
    perMessageDeflate: config.wsCompression ?? false,
    maxPayload: config.wsMaxPayload ?? 1024 * 1024,
  });

  // Create rate limiter
  const rateLimiter = new ConnectionRateLimiter({
    maxConnectionsPerSecond: config.maxConnectionsPerSecond ?? 100,
    maxPendingConnections: config.maxPendingConnections ?? 1000,
    cooldownMs: 1000,
  });

  // Create rate-limited logger
  const rateLimitedLogger = new RateLimitedLogger({
    windowMs: 10000,
    maxPerWindow: 5
  });

  return {
    httpServer,
    wss,
    rateLimiter,
    rateLimitedLogger,
    // DEFERRED STARTUP - call this AFTER ServerCoordinator assembly
    start: () => {
      return new Promise<number>((resolve) => {
        httpServer.listen(config.port, () => {
          const actualPort = (httpServer.address() as any).port;
          logger.info({ port: actualPort }, 'Server Coordinator listening');
          resolve(actualPort);
        });
      });
    },
    // Deferred wiring: allows ServerFactory to inject the /sync handler
    // after HttpSyncHandler is assembled
    setHttpRequestHandler: (handler: (req: any, res: any) => void) => {
      currentRequestHandler = handler;
    },
  };
}
