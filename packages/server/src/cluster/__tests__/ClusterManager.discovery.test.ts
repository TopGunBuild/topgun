import { ClusterManager } from '../ClusterManager';
import * as dns from 'dns';
import { EventEmitter } from 'events';

// Mock logger with proper structure
jest.mock('../../utils/logger', () => {
  const mockLogger: Record<string, jest.Mock> = {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn(),
    trace: jest.fn(),
    child: jest.fn(),
  };
  mockLogger.child.mockReturnValue(mockLogger);
  return { logger: mockLogger };
});

// Mock WebSocket and WebSocketServer
const mockWebSocketInstances: any[] = [];
const mockWebSocketServerInstances: any[] = [];

jest.mock('ws', () => {
  const MockWebSocket = jest.fn().mockImplementation(() => {
    const ws = new EventEmitter();
    (ws as any).send = jest.fn();
    (ws as any).close = jest.fn();
    (ws as any).terminate = jest.fn();
    (ws as any).readyState = 1; // OPEN
    mockWebSocketInstances.push(ws);
    // Simulate connection after a tick
    setImmediate(() => ws.emit('open'));
    return ws;
  });

  const MockWebSocketServer = jest.fn().mockImplementation((options: any) => {
    const wss = new EventEmitter();
    (wss as any).close = jest.fn();
    (wss as any).address = jest.fn().mockReturnValue({ port: options.port || 9080 });
    mockWebSocketServerInstances.push(wss);
    // Simulate listening after a tick
    setImmediate(() => wss.emit('listening'));
    return wss;
  });

  return {
    WebSocket: MockWebSocket,
    WebSocketServer: MockWebSocketServer,
    default: MockWebSocket,
  };
});

// Mock DNS
jest.mock('dns', () => ({
  promises: {
    resolve4: jest.fn()
  }
}));

describe('ClusterManager - Kubernetes Discovery', () => {
  let clusterManager: ClusterManager;
  const mockResolve4 = dns.promises.resolve4 as jest.Mock;
  const MOCK_PORT = 9080;

  beforeEach(() => {
    jest.clearAllMocks();
    mockWebSocketInstances.length = 0;
    mockWebSocketServerInstances.length = 0;

    clusterManager = new ClusterManager({
      nodeId: 'test-node',
      host: 'localhost',
      port: MOCK_PORT,
      peers: [],
      discovery: 'kubernetes',
      serviceName: 'topgun-headless',
      discoveryInterval: 1000 // fast interval for test
    });
  });

  afterEach(() => {
    clusterManager.stop();
  });

  it('should resolve DNS and connect to discovered peers', async () => {
    mockResolve4.mockResolvedValue(['10.0.0.1', '10.0.0.2']);

    // Start cluster (which starts discovery)
    await clusterManager.start();

    // Wait for discovery interval and async operations
    await new Promise(resolve => setTimeout(resolve, 50));

    expect(mockResolve4).toHaveBeenCalledWith('topgun-headless');

    // Check WebSocket was instantiated for peers
    const { WebSocket } = require('ws');
    // 2 calls for peers (WebSocketServer is separate)
    expect(WebSocket).toHaveBeenCalledWith(`ws://10.0.0.1:${MOCK_PORT}`);
    expect(WebSocket).toHaveBeenCalledWith(`ws://10.0.0.2:${MOCK_PORT}`);
  });

  it('should handle DNS resolution failures gracefully', async () => {
    mockResolve4.mockRejectedValue(new Error('ENOTFOUND'));

    await clusterManager.start();

    // Wait for discovery to run
    await new Promise(resolve => setTimeout(resolve, 50));

    // Should not crash, should log error
    expect(mockResolve4).toHaveBeenCalled();

    // Verify logger.error was called
    const { logger } = require('../../utils/logger');
    expect(logger.error).toHaveBeenCalled();
  });

  it('should use actual port for peer connections if available', async () => {
    mockResolve4.mockResolvedValue(['10.0.0.3']);

    await clusterManager.start();

    // Wait for discovery
    await new Promise(resolve => setTimeout(resolve, 50));

    const { WebSocket } = require('ws');
    expect(WebSocket).toHaveBeenCalledWith(`ws://10.0.0.3:${MOCK_PORT}`);
  });
});

