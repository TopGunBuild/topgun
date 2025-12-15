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
│   ├── smoke.js            # Basic smoke test
│   ├── connection-storm.js # Massive concurrent connections
│   ├── write-heavy.js      # Intensive write operations
│   ├── read-heavy.js       # Mass subscriptions test
│   └── mixed-workload.js   # Realistic production load
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
k6 run tests/k6/scenarios/smoke.js -e JWT_TOKEN=<token>
```

#### Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| `topgun_connection_time` | p(95) < 1000ms | Connection establishment time |
| `topgun_auth_time` | p(95) < 500ms | Authentication time |
| `topgun_auth_success` | rate > 99% | Authentication success rate |
| `topgun_errors` | count < 10 | Total errors |

---

### Connection Storm (`scenarios/connection-storm.js`)

Stress test for massive concurrent connections with ramping VUs.

- **Ramping**: 0 → 100 → 500 → 1000 VUs over 5 minutes
- **Operations**: Connect → Authenticate → Hold connection → Disconnect
- **Tests**: Server's ability to handle connection spikes

```bash
# Full test
k6 run tests/k6/scenarios/connection-storm.js -e JWT_TOKEN=<token>

# Debug mode (lower load)
k6 run tests/k6/scenarios/connection-storm.js -e JWT_TOKEN=<token> --vus 10 --duration 30s
```

#### Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| `connection_error_rate` | rate < 5% | Connection failures |
| `topgun_connection_time` | p(95) < 500ms | Connection time |
| `topgun_auth_success` | rate > 95% | Authentication success |

#### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HOLD_TIME` | `5000` | Connection hold time in ms |

#### Success Metrics

- 1000 VUs without critical errors
- < 5% connection failures
- p95 connection time < 500ms

---

### Write-Heavy (`scenarios/write-heavy.js`)

Intensive write load test to measure PUT operation throughput.

- **VUs**: 100 virtual users
- **Duration**: 5 minutes
- **Operations**: ~10 PUT operations per second per VU
- **Tests**: Write throughput and latency under load

```bash
# Full test
k6 run tests/k6/scenarios/write-heavy.js -e JWT_TOKEN=<token>

# Debug mode
k6 run tests/k6/scenarios/write-heavy.js -e JWT_TOKEN=<token> --vus 10 --duration 30s
```

#### Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| `write_latency` | p(99) < 100ms | Write operation latency |
| `write_error_rate` | rate < 1% | Write failures |
| `write_ops_total` | count > 300,000 | Total operations |

#### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `OPS_PER_SECOND` | `10` | Write ops per second per VU |
| `BATCH_SIZE` | `5` | Operations per batch |

#### Success Metrics

- > 1000 ops/sec sustained throughput
- p99 latency < 100ms
- < 1% error rate

---

### Read-Heavy (`scenarios/read-heavy.js`)

Test for massive subscriptions and update propagation.

- **VUs**: 200 virtual users
- **Duration**: 3 minutes
- **Readers**: 90% of VUs subscribe to 5 maps each
- **Writers**: 10% of VUs generate continuous updates
- **Tests**: Subscription handling and update propagation

```bash
# Full test
k6 run tests/k6/scenarios/read-heavy.js -e JWT_TOKEN=<token>

# Debug mode
k6 run tests/k6/scenarios/read-heavy.js -e JWT_TOKEN=<token> --vus 20 --duration 30s
```

#### Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| `subscription_latency` | p(95) < 200ms | Subscription setup time |
| `update_propagation_time` | p(95) < 50ms | Update delivery time |
| `topgun_errors` | rate < 2% | Total errors |

#### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MAPS_PER_VU` | `5` | Subscriptions per reader |
| `WRITER_PERCENTAGE` | `10` | Percentage of writer VUs |
| `WRITES_PER_SECOND` | `5` | Writes per second per writer |

#### Success Metrics

- 1000 active subscriptions
- Update propagation < 50ms p95
- < 2% error rate

---

### Mixed Workload (`scenarios/mixed-workload.js`)

Realistic production-like load with mixed read/write operations.

- **VUs**: 150 (ramping: 0 → 150 → 0)
- **Duration**: 10 minutes
- **Readers**: 70% of VUs (subscribe to 3-7 maps)
- **Writers**: 30% of VUs (10 ops/sec)
- **Tests**: End-to-end latency and sustained throughput

```bash
# Full test
k6 run tests/k6/scenarios/mixed-workload.js -e JWT_TOKEN=<token>

# Debug mode
k6 run tests/k6/scenarios/mixed-workload.js -e JWT_TOKEN=<token> --vus 15 --duration 1m
```

#### Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| `e2e_latency` | p(95) < 100ms | End-to-end latency |
| `write_latency` | p(95) < 50ms | Write operation latency |
| `error_rate` | rate < 2% | Total error rate |
| `total_operations` | count > 300,000 | Total operations |

#### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `READER_PERCENTAGE` | `70` | Percentage of reader VUs |
| `WRITE_RATE` | `10` | Writes per second per writer |
| `MAPS_COUNT` | `20` | Number of maps to use |

#### Success Metrics

- Stable operation for 10 minutes
- > 500 ops/sec sustained throughput
- < 2% error rate
- p95 e2e latency < 100ms

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
