# TopGun k6 Load Testing

Load testing suite for TopGun server using [k6](https://k6.io/).

## Prerequisites

1. Install k6:
   ```bash
   # macOS
   brew install k6

   # Linux (Debian/Ubuntu)
   sudo gpg -k
   sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69
   echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
   sudo apt-get update
   sudo apt-get install k6

   # Windows
   choco install k6
   ```

2. Start TopGun server:
   ```bash
   cd /path/to/topgun
   pnpm start:server
   ```

## Quick Start

Run smoke test against local server:

```bash
k6 run tests/k6/scenarios/smoke.js
```

With custom server URL:

```bash
k6 run tests/k6/scenarios/smoke.js -e WS_URL=ws://localhost:3000
```

## Directory Structure

```
tests/k6/
├── lib/
│   └── topgun-client.js    # TopGun WebSocket client library
├── scenarios/
│   └── smoke.js            # Basic smoke test
├── results/                # Test results (gitignored)
└── README.md
```

## Available Scenarios

### Smoke Test (`scenarios/smoke.js`)

Basic connectivity and functionality test.

- **VUs**: 10 virtual users
- **Duration**: 30 seconds
- **Operations**: Connect → Authenticate → PUT → Disconnect

```bash
k6 run tests/k6/scenarios/smoke.js
```

#### Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| `topgun_connection_time` | p(95) < 1000ms | Connection establishment time |
| `topgun_auth_time` | p(95) < 500ms | Authentication time |
| `topgun_auth_success` | rate > 99% | Authentication success rate |
| `topgun_errors` | count < 10 | Total errors |

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `WS_URL` | `ws://localhost:8080` | WebSocket server URL |
| `JWT_TOKEN` | (none) | Pre-generated JWT token |

### Using Pre-generated JWT Tokens

For load testing, you **must** provide a valid JWT token. The default server secret is `topgun-secret-dev`:

```bash
# Generate token (example using Node.js)
node -e "
const jwt = require('jsonwebtoken');
const token = jwt.sign(
  { userId: 'k6-user', roles: ['ADMIN'], sub: 'k6-user' },
  'topgun-secret-dev',  // Default dev secret
  { expiresIn: '24h' }
);
console.log(token);
"

# Use the token
k6 run tests/k6/scenarios/smoke.js -e JWT_TOKEN=<generated-token>
```

For production servers, use the appropriate `JWT_SECRET` environment variable or server configuration.

## Custom Metrics

The test suite tracks these custom metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `topgun_connection_time` | Trend | Time to establish WebSocket connection |
| `topgun_auth_time` | Trend | Time for authentication handshake |
| `topgun_message_latency` | Trend | Round-trip PING/PONG latency |
| `topgun_messages_sent` | Counter | Total messages sent |
| `topgun_messages_received` | Counter | Total messages received |
| `topgun_auth_success` | Rate | Authentication success rate |
| `topgun_errors` | Counter | Total errors |

## Client Library Usage

The `lib/topgun-client.js` provides a reusable client for custom scenarios:

```javascript
import ws from 'k6/ws';
import { TopGunClient, createMessageHandler } from '../lib/topgun-client.js';

export default function() {
  ws.connect('ws://localhost:8080', {}, function(socket) {
    const client = new TopGunClient(socket, 'my-node-id');

    const handler = createMessageHandler(client, {
      onAuthRequired: () => {
        client.authenticate(myToken);
      },
      onAuthAck: () => {
        // Authenticated - perform operations
        client.putBatch([{ mapName: 'users', key: 'user-1', value: { name: 'John' } }]);
        client.subscribe('users', {});
        client.ping();
      },
      onOpAck: () => {
        // Operation acknowledged - safe to close
        socket.close();
      },
      onQueryResponse: (msg) => {
        console.log('Query results:', msg.payload);
      },
    });

    // IMPORTANT: Use 'binaryMessage' for MessagePack responses
    socket.on('binaryMessage', handler);
  });
}
```

## Protocol Reference

All messages are sent as JSON (server supports JSON fallback):

### AUTH
```json
{ "type": "AUTH", "token": "<JWT>" }
```

### OP_BATCH (PUT - recommended)
Use `OP_BATCH` for operations when you need acknowledgment:
```json
{
  "type": "OP_BATCH",
  "payload": {
    "ops": [{
      "id": "<unique-op-id>",
      "mapName": "<map-name>",
      "opType": "PUT",
      "key": "<key>",
      "record": {
        "value": { ... },
        "timestamp": {
          "millis": 1234567890,
          "counter": 0,
          "nodeId": "<node-id>"
        }
      }
    }]
  }
}
```
Server responds with `OP_ACK` containing `{ lastId: "<last-processed-op-id>" }`.

### CLIENT_OP (PUT - fire and forget)
```json
{
  "type": "CLIENT_OP",
  "payload": {
    "id": "<unique-op-id>",
    "mapName": "<map-name>",
    "opType": "PUT",
    "key": "<key>",
    "record": {
      "value": { ... },
      "timestamp": {
        "millis": 1234567890,
        "counter": 0,
        "nodeId": "<node-id>"
      }
    }
  }
}
```
Note: `CLIENT_OP` does not receive acknowledgment (only `OP_REJECTED` on error).

### QUERY_SUB
```json
{
  "type": "QUERY_SUB",
  "payload": {
    "queryId": "<id>",
    "mapName": "<map>",
    "query": {}
  }
}
```

### PING
```json
{ "type": "PING", "timestamp": 1234567890 }
```

## Troubleshooting

### Connection refused
Ensure the TopGun server is running on the expected port.

### Authentication failures
- Check that the JWT secret matches between k6 and server
- Use pre-generated tokens with `-e JWT_TOKEN=...`

### Timeout errors
- Server may be under heavy load
- Increase timeout in scenario configuration

## Creating New Scenarios

1. Create a new file in `scenarios/`
2. Import the client library
3. Define options and test function

```javascript
import ws from 'k6/ws';
import { TopGunClient, createMessageHandler } from '../lib/topgun-client.js';

export const options = {
  vus: 50,
  duration: '5m',
};

export default function() {
  // Your test logic
}
```

## References

- [k6 Documentation](https://k6.io/docs/)
- [k6 WebSocket API](https://k6.io/docs/javascript-api/k6-ws/)
- TopGun E2E tests: `tests/e2e/json-fallback.test.ts`
