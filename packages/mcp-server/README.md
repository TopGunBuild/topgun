# @topgunbuild/mcp-server

MCP (Model Context Protocol) Server for TopGun - enables AI assistants like Claude Desktop and Cursor to interact with TopGun databases through natural language.

## What is MCP?

MCP (Model Context Protocol) is Anthropic's open protocol for connecting AI assistants to external data sources. It allows AI tools to query, modify, and search your TopGun database using natural language.

## Installation

```bash
npm install @topgunbuild/mcp-server
# or
pnpm add @topgunbuild/mcp-server
```

## Quick Start

### Claude Desktop

1. Install the MCP server globally:
```bash
npm install -g @topgunbuild/mcp-server
```

2. Configure Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "topgun": {
      "command": "npx",
      "args": ["@topgunbuild/mcp-server"],
      "env": {
        "TOPGUN_URL": "ws://localhost:8080"
      }
    }
  }
}
```

3. Restart Claude Desktop

4. Try: "Show me all tasks in my TopGun database"

### Cursor

Add to `.cursor/config.json` in your workspace:

```json
{
  "mcp": {
    "servers": {
      "topgun": {
        "command": "npx",
        "args": ["@topgunbuild/mcp-server"],
        "env": {
          "TOPGUN_URL": "ws://localhost:8080"
        }
      }
    }
  }
}
```

### Programmatic Usage

```typescript
import { TopGunMCPServer } from '@topgunbuild/mcp-server';

const server = new TopGunMCPServer({
  topgunUrl: 'ws://localhost:8080',
  allowedMaps: ['tasks', 'users'],
  enableMutations: true,
  debug: true,
});

await server.start();
```

## CLI Options

```bash
topgun-mcp [options]

Options:
  --url, -u <url>         TopGun server URL (default: ws://localhost:8080)
  --token, -t <token>     Authentication token
  --maps, -m <maps>       Comma-separated list of allowed maps
  --no-mutations          Disable mutation operations
  --no-subscriptions      Disable subscription operations
  --http                  Start HTTP transport instead of stdio
  --port, -p <port>       HTTP port (default: 3000)
  --host <ip>             HTTP bind address (default: 127.0.0.1)
  --allow-public-bind     Allow a non-loopback --host
  --debug, -d             Enable debug logging
  --help, -h              Show help
  --version, -v           Show version
```

## Available Tools

### topgun_list_maps
List all available maps.

### topgun_query
Query data from a map with filters and sorting.

```
Example: "Show me all tasks with status 'done'"
```

### topgun_mutate
Create, update, or delete data.

```
Example: "Create a task called 'Review PR #123'"
```

### topgun_search
Tri-hybrid search combining exact, full-text (BM25), and semantic methods fused with Reciprocal Rank Fusion. Defaults to full-text only.

**Parameters:**
- `map` (required) — name of the map to search
- `query` (required) — search query text
- `methods` (optional, default `["fullText"]`) — array of methods to combine: `"exact"`, `"fullText"`, `"semantic"`. `"semantic"` requires server-side auto-embedding.
- `limit` (optional, default 10) — maximum results to return (maps to `k` in the hybrid search engine internally)
- `minScore` (optional, default 0) — minimum fused relevance score (0–1)

Returns results ranked by fused score with per-method score breakdown.

```
Example: "Find tasks about authentication"
Example with methods: { map: "docs", query: "auth flow", methods: ["exact", "fullText"] }
```

### topgun_subscribe
Watch a map for real-time changes.

```
Example: "Watch for new high-priority tasks"
```

### topgun_schema
Get schema information about a map.

```
Example: "What fields does the tasks map have?"
```

### topgun_stats
Get statistics about TopGun connection and maps.

```
Example: "What's the status of my TopGun connection?"
```

### topgun_explain
Explain how a query would be executed.

```
Example: "Explain query on tasks where status='done'"
```

## Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `topgunUrl` | string | `ws://localhost:8080` | TopGun server WebSocket URL |
| `authToken` | string | - | Authentication token |
| `allowedMaps` | string[] | - | Restrict to specific maps (all by default) |
| `enableMutations` | boolean | `true` | Allow write operations |
| `enableSubscriptions` | boolean | `true` | Allow subscriptions |
| `defaultLimit` | number | `10` | Default query result limit |
| `maxLimit` | number | `100` | Maximum query result limit |
| `subscriptionTimeoutSeconds` | number | `60` | Subscription timeout |
| `debug` | boolean | `false` | Enable debug logging |

## HTTP Transport

For MCP clients over the network, you can start the server with HTTP transport; it serves only `tools/call`. It needs a secret that every request
presents, and it listens on `127.0.0.1` unless you allow more:

```bash
export TOPGUN_MCP_HTTP_TOKEN=$(openssl rand -base64 32)
topgun-mcp --http --port 3000
curl -H "Authorization: Bearer $TOPGUN_MCP_HTTP_TOKEN" http://127.0.0.1:3000/health
```

- `TOPGUN_MCP_HTTP_TOKEN` (environment only) must be at least 32 characters of `A-Z a-z 0-9 - . _ ~ + /`, optionally
  followed by `=` padding. Without it, or with a short or malformed value, the start is refused.
- Every request, `/health` included, must send `Authorization: Bearer <secret>`; otherwise the answer is `401` and
  nothing runs.
- `--host <ip>` / `TOPGUN_MCP_HOST` takes an IP address, not a name. An address outside `127.0.0.0/8` and `::1` also
  needs `--allow-public-bind` / `TOPGUN_MCP_ALLOW_PUBLIC_BIND=1`; the server then logs a warning. The transport is
  plain HTTP: put TLS in front of it when it is reachable from the network.
- An authenticated request with an `Origin` header gets `403` unless the origin is allowlisted (a request without
  the secret gets the same `401` whatever its `Origin`). The CLI allowlists none, on purpose; pass `corsOrigins` to
  `HTTPTransport` for a browser deployment. Each entry is a serialized origin such as `https://app.example.com`
  (no path, no trailing slash, no default port); `*` is refused.
- An unsafe or malformed setting (secret, host, or a port that is not an integer from 1 to 65535) exits with code 1
  and one line on stderr: `topgun-mcp: refusing to start: …`. `HTTPTransport` applies the same port rule.

Library use: `new HTTPTransport({ bearerToken, host?, allowPublicBind?, port?, corsOrigins? })`, then
`start(server)`. The constructor throws `HTTPTransportConfigError` (with a `field`) on the settings above.

Endpoints:
- `GET /health` - Health check
- `GET /mcp` - Server info
- `POST /mcp` - Stateless tool calls (`tools/call`); this is the only MCP method the HTTP transport serves
- `GET /mcp/events` - opens an SSE stream, but SSE session messaging is not supported yet

## Environment Variables

| Variable | Description |
|----------|-------------|
| `TOPGUN_URL` | TopGun server URL |
| `TOPGUN_TOKEN` | Authentication token |
| `TOPGUN_MAPS` | Comma-separated allowed maps |
| `TOPGUN_MCP_PORT` | HTTP port (with --http), an integer from 1 to 65535 |
| `TOPGUN_MCP_HTTP_TOKEN` | Required with --http: the secret every HTTP request must send as `Authorization: Bearer` |
| `TOPGUN_MCP_HOST` | HTTP bind address, an IP literal (default `127.0.0.1`) |
| `TOPGUN_MCP_ALLOW_PUBLIC_BIND` | `1` or `true` allows a non-loopback `TOPGUN_MCP_HOST` |
| `TOPGUN_DEBUG` | Enable debug (true/false) |

## Security

- Use `allowedMaps` to restrict access to specific maps
- Use `enableMutations: false` for read-only access
- Use authentication tokens in production
- Over HTTP, every holder of `TOPGUN_MCP_HTTP_TOKEN` acts as the TopGun identity of `--token` / `TOPGUN_TOKEN`; a
  separate identity per HTTP caller is not yet supported
- Keep the HTTP transport on loopback, or put TLS in front of it: plain HTTP sends the bearer secret in clear

## License

Apache-2.0
