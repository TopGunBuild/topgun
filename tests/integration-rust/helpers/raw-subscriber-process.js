/**
 * A protocol-level live subscriber that runs in a process of its own, so that
 * how fast it reads its socket does not depend on what the test process is
 * doing. A real subscriber never shares its event loop with the writer.
 *
 * Started with `child_process.fork(thisFile, [port, mapName])`. It
 * authenticates, holds a live query of the whole map, and counts the
 * SERVER_EVENTs it receives for that map. Messages to the parent:
 * `{ type: 'ready' }` once the live query's first QUERY_RESP has arrived,
 * `{ type: 'closed' }` if the socket closes, and `{ type: 'final', keys,
 * events, closed }` in answer to `{ type: 'final' }`, `keys` being the number
 * of keys a fresh query of the map returns. `{ type: 'exit' }` ends it.
 *
 * Plain JavaScript on the root dependencies (msgpackr, ws, jsonwebtoken), the
 * same encoding `@topgunbuild/core` uses minus its `undefined` stripping, which
 * nothing sent here needs; so it runs without building any package.
 */
const { pack, unpack } = require('msgpackr');
const jwt = require('jsonwebtoken');
const WebSocket = require('ws');

const [port, mapName] = process.argv.slice(2);
const token = jwt.sign({ sub: 'raw-subscriber', roles: ['ADMIN'] }, 'test-e2e-secret', {
  expiresIn: '1h',
});

let events = 0;
let closed = false;
let finalAsked = false;
const ws = new WebSocket(`ws://localhost:${port}/ws`);
ws.binaryType = 'arraybuffer';
const send = (message) => ws.readyState === WebSocket.OPEN && ws.send(pack(message));

const heartbeat = setInterval(() => send({ type: 'PING', timestamp: Date.now() }), 5_000);

ws.on('message', (data) => {
  const message = unpack(new Uint8Array(data));
  switch (message.type) {
    case 'AUTH_REQUIRED':
      send({ type: 'AUTH', token });
      break;
    case 'AUTH_ACK':
      send({ type: 'QUERY_SUB', payload: { queryId: 'live', mapName, query: {} } });
      break;
    case 'SERVER_EVENT':
      if (message.payload?.mapName === mapName) events += 1;
      break;
    case 'SERVER_BATCH_EVENT':
      for (const event of message.payload?.events ?? []) {
        if (event.mapName === mapName) events += 1;
      }
      break;
    case 'QUERY_RESP':
      if (message.payload?.queryId === 'live') process.send({ type: 'ready' });
      if (message.payload?.queryId === 'final' && finalAsked) {
        const keys = new Set((message.payload.results ?? []).map((r) => r.key)).size;
        process.send({ type: 'final', keys, events, closed });
      }
      break;
    default:
      break;
  }
});
ws.on('close', () => {
  closed = true;
  process.send({ type: 'closed' });
});
ws.on('error', () => undefined);

process.on('message', (message) => {
  if (message?.type === 'final') {
    finalAsked = true;
    send({ type: 'QUERY_SUB', payload: { queryId: 'final', mapName, query: {} } });
  } else if (message?.type === 'exit') {
    clearInterval(heartbeat);
    ws.close();
    process.exit(0);
  }
});
