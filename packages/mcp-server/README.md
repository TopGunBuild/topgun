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
Perform hybrid search (BM25 full-text + exact matching).

```
Example: "Find tasks about authentication"
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

For web-based MCP clients, you can start the server with HTTP transport:

```bash
topgun-mcp --http --port 3000
```

Endpoints:
- `GET /health` - Health check
- `GET /mcp` - Server info
- `GET /mcp/events` - SSE connection
- `POST /mcp` - Stateless tool calls

## Environment Variables

| Variable | Description |
|----------|-------------|
| `TOPGUN_URL` | TopGun server URL |
| `TOPGUN_TOKEN` | Authentication token |
| `TOPGUN_MAPS` | Comma-separated allowed maps |
| `TOPGUN_MCP_PORT` | HTTP port (with --http) |
| `TOPGUN_DEBUG` | Enable debug (true/false) |

## Security

- Use `allowedMaps` to restrict access to specific maps
- Use `enableMutations: false` for read-only access
- Use authentication tokens in production
- The MCP server runs locally - data never leaves your machine

## License

BSL-1.1
