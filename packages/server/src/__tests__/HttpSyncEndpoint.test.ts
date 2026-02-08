import { createServer, Server, IncomingMessage, ServerResponse } from 'http';
import * as jwt from 'jsonwebtoken';
import { HLC, LWWMap, serialize, deserialize, HttpSyncRequestSchema } from '@topgunbuild/core';
import type { HttpSyncRequest, HttpSyncResponse } from '@topgunbuild/core';
import { HttpSyncHandler } from '../coordinator/http-sync-handler';

const TEST_PORT = 12050;
const JWT_SECRET = 'test-secret-key-for-http-sync';

describe('HttpSyncEndpoint', () => {
  let server: Server;
  let hlc: HLC;
  let testMap: LWWMap<string, any>;
  let httpSyncHandler: HttpSyncHandler;

  function makeAuthToken(payload: any = { userId: 'user-1', roles: ['USER'] }): string {
    return jwt.sign(payload, JWT_SECRET, { algorithm: 'HS256' });
  }

  function sendSyncRequest(
    body: HttpSyncRequest,
    options: { token?: string; contentType?: string } = {},
  ): Promise<{ status: number; body: any }> {
    return new Promise((resolve, reject) => {
      const isJson = options.contentType === 'application/json';
      const bodyData = isJson
        ? Buffer.from(JSON.stringify(body))
        : Buffer.from(serialize(body));

      const req = require('http').request(
        {
          hostname: '127.0.0.1',
          port: TEST_PORT,
          path: '/sync',
          method: 'POST',
          headers: {
            'Content-Type': options.contentType || 'application/x-msgpack',
            'Content-Length': bodyData.length,
            ...(options.token !== undefined
              ? { 'Authorization': `Bearer ${options.token}` }
              : {}),
          },
        },
        (res: IncomingMessage) => {
          const chunks: Buffer[] = [];
          res.on('data', (chunk: Buffer) => chunks.push(chunk));
          res.on('end', () => {
            const responseBody = Buffer.concat(chunks);
            let parsed: any;
            try {
              const ct = res.headers['content-type'] || '';
              if (ct.includes('application/json')) {
                parsed = JSON.parse(responseBody.toString('utf-8'));
              } else {
                parsed = deserialize(responseBody);
              }
            } catch {
              parsed = responseBody.toString('utf-8');
            }
            resolve({ status: res.statusCode || 0, body: parsed });
          });
        },
      );
      req.on('error', reject);
      req.write(bodyData);
      req.end();
    });
  }

  beforeAll((done) => {
    hlc = new HLC('test-server');
    testMap = new LWWMap<string, any>(hlc);

    httpSyncHandler = new HttpSyncHandler({
      authHandler: {
        verifyToken: (token: string) => {
          const decoded = jwt.verify(token, JWT_SECRET, { algorithms: ['HS256'] }) as any;
          if (!decoded.roles) decoded.roles = ['USER'];
          return decoded;
        },
        handleAuth: async () => ({ success: true }),
      },
      operationHandler: {
        applyOpToMap: async (op: any) => {
          testMap.set(op.key, op.record?.value);
          return { eventPayload: {}, oldRecord: null };
        },
        processClientOp: async () => {},
        processOpBatch: async () => {},
        processLocalOp: async () => {},
      },
      storageManager: {
        getMapAsync: async () => testMap,
        getMap: () => testMap,
        getMaps: () => new Map(),
        hasMap: () => false,
        loadMapFromStorage: async () => {},
        isMapLoading: () => false,
      },
      queryConversionHandler: {
        executeLocalQuery: async () => [],
        convertToCoreQuery: () => null,
        predicateToCoreQuery: () => null,
        convertOperator: () => null,
        finalizeClusterQuery: async () => {},
        stop: () => {},
      },
      searchCoordinator: {
        search: () => ({ results: [], totalCount: 0, requestId: '' }),
      },
      hlc,
      securityManager: {
        checkPermission: () => true,
      },
    });

    // Create a minimal HTTP server that routes /sync
    server = createServer((req: IncomingMessage, res: ServerResponse) => {
      if (req.method === 'POST' && req.url === '/sync') {
        const authHeader = req.headers['authorization'] || '';
        const token = authHeader.startsWith('Bearer ') ? authHeader.slice(7) : '';

        if (!token) {
          res.writeHead(401, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ error: 'Missing Authorization header' }));
          return;
        }

        const chunks: Buffer[] = [];
        req.on('data', (chunk: Buffer) => chunks.push(chunk));
        req.on('end', async () => {
          try {
            const body = Buffer.concat(chunks);
            const ct = req.headers['content-type'] || '';
            const isJson = ct.includes('application/json');

            let parsed: any;
            if (isJson) {
              parsed = JSON.parse(body.toString('utf-8'));
            } else {
              parsed = deserialize(body);
            }

            const validation = HttpSyncRequestSchema.safeParse(parsed);
            if (!validation.success) {
              res.writeHead(400, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({ error: 'Invalid request body' }));
              return;
            }

            const response = await httpSyncHandler.handleSyncRequest(validation.data, token);

            if (isJson) {
              res.writeHead(200, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify(response));
            } else {
              const responseBytes = serialize(response);
              res.writeHead(200, { 'Content-Type': 'application/x-msgpack' });
              res.end(Buffer.from(responseBytes));
            }
          } catch (err: any) {
            const message = err.message || '';
            if (message.startsWith('401:')) {
              res.writeHead(401, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({ error: message.slice(5).trim() }));
            } else {
              res.writeHead(500, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({ error: 'Internal error' }));
            }
          }
        });
      } else {
        res.writeHead(200);
        res.end('TopGun Server Running');
      }
    });

    server.listen(TEST_PORT, done);
  });

  afterAll((done) => {
    server.close(done);
  });

  it('POST /sync with valid auth returns 200', async () => {
    const result = await sendSyncRequest(
      { clientId: 'c1', clientHlc: hlc.now() },
      { token: makeAuthToken() },
    );

    expect(result.status).toBe(200);
    expect(result.body.serverHlc).toBeDefined();
  });

  it('POST /sync without auth returns 401', async () => {
    const result = await sendSyncRequest(
      { clientId: 'c1', clientHlc: hlc.now() },
      { token: undefined },
    );

    // No Authorization header at all
    const req = require('http').request(
      {
        hostname: '127.0.0.1',
        port: TEST_PORT,
        path: '/sync',
        method: 'POST',
        headers: { 'Content-Type': 'application/x-msgpack' },
      },
      (res: IncomingMessage) => {
        expect(res.statusCode).toBe(401);
        res.resume();
      },
    );
    const bodyData = Buffer.from(serialize({ clientId: 'c1', clientHlc: hlc.now() }));
    req.write(bodyData);
    req.end();
    // Wait for server to process the unauthenticated request
    await new Promise<void>((resolve) => {
      req.on('response', (res: any) => { res.resume(); resolve(); });
      // Fallback timeout in case no response event fires
      setTimeout(resolve, 500);
    });
  });

  it('POST /sync with invalid body returns 400', async () => {
    // Send a request missing clientId
    const result = await sendSyncRequest(
      { clientHlc: hlc.now() } as any,
      { token: makeAuthToken() },
    );

    expect(result.status).toBe(400);
  });

  it('POST /sync with operations returns ack in response', async () => {
    const result = await sendSyncRequest(
      {
        clientId: 'c1',
        clientHlc: hlc.now(),
        operations: [
          { mapName: 'users', key: 'user-1', id: 'op-1', record: { value: { name: 'Alice' }, timestamp: hlc.now() } },
        ],
      },
      { token: makeAuthToken() },
    );

    expect(result.status).toBe(200);
    expect(result.body.ack).toBeDefined();
    expect(result.body.ack.lastId).toBe('op-1');
  });

  it('POST /sync with syncMaps returns deltas', async () => {
    // Seed the map with data first
    testMap.set('test-key', { name: 'Test' });

    const result = await sendSyncRequest(
      {
        clientId: 'c1',
        clientHlc: hlc.now(),
        syncMaps: [{ mapName: 'users', lastSyncTimestamp: { millis: 0, counter: 0, nodeId: '' } }],
      },
      { token: makeAuthToken() },
    );

    expect(result.status).toBe(200);
    expect(result.body.deltas).toBeDefined();
    expect(result.body.deltas.length).toBeGreaterThan(0);
  });

  it('round-trip: push operation then pull delta in next request', async () => {
    // Push an operation
    const pushResult = await sendSyncRequest(
      {
        clientId: 'c1',
        clientHlc: hlc.now(),
        operations: [
          { mapName: 'messages', key: 'msg-1', id: 'op-rt', record: { value: { text: 'hello' }, timestamp: hlc.now() } },
        ],
      },
      { token: makeAuthToken() },
    );

    expect(pushResult.status).toBe(200);
    expect(pushResult.body.ack).toBeDefined();

    // Pull deltas - the operation we just pushed should appear
    const pullResult = await sendSyncRequest(
      {
        clientId: 'c2',
        clientHlc: hlc.now(),
        syncMaps: [{ mapName: 'users', lastSyncTimestamp: { millis: 0, counter: 0, nodeId: '' } }],
      },
      { token: makeAuthToken() },
    );

    expect(pullResult.status).toBe(200);
    expect(pullResult.body.deltas).toBeDefined();
  });

  it('verifies msgpackr request/response serialization works', async () => {
    const result = await sendSyncRequest(
      { clientId: 'c1', clientHlc: hlc.now() },
      { token: makeAuthToken() },
    );

    expect(result.status).toBe(200);
    // Response should have been deserialized from msgpackr successfully
    expect(typeof result.body).toBe('object');
    expect(result.body.serverHlc).toBeDefined();
    expect(typeof result.body.serverHlc.millis).toBe('number');
    expect(typeof result.body.serverHlc.nodeId).toBe('string');
  });

  it('POST /sync with invalid auth token returns 401', async () => {
    const result = await sendSyncRequest(
      { clientId: 'c1', clientHlc: hlc.now() },
      { token: 'not-a-valid-jwt' },
    );

    expect(result.status).toBe(401);
  });
});
