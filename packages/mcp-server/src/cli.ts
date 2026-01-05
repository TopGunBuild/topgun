/**
 * TopGun MCP Server CLI
 *
 * Start the MCP server from command line.
 *
 * Usage:
 *   topgun-mcp [options]
 *
 * Options:
 *   --url <url>           TopGun server URL (default: ws://localhost:8080)
 *   --token <token>       Authentication token
 *   --maps <maps>         Comma-separated list of allowed maps
 *   --no-mutations        Disable mutation operations
 *   --no-subscriptions    Disable subscription operations
 *   --http                Start HTTP transport instead of stdio
 *   --port <port>         HTTP port (default: 3000)
 *   --debug               Enable debug logging
 *   --help                Show help
 *   --version             Show version
 */

import { TopGunMCPServer } from './TopGunMCPServer';
import { HTTPTransport } from './transport/http';
import type { MCPServerConfig } from './types';
import { createLogger } from './logger';

interface CLIOptions {
  url: string;
  token?: string;
  maps?: string[];
  mutations: boolean;
  subscriptions: boolean;
  http: boolean;
  port: number;
  debug: boolean;
  help: boolean;
  version: boolean;
}

const VERSION = '0.8.1';

function parseArgs(args: string[]): CLIOptions {
  const options: CLIOptions = {
    url: process.env.TOPGUN_URL || 'ws://localhost:8080',
    token: process.env.TOPGUN_TOKEN,
    maps: process.env.TOPGUN_MAPS?.split(',').map((m) => m.trim()),
    mutations: true,
    subscriptions: true,
    http: false,
    port: parseInt(process.env.TOPGUN_MCP_PORT || '3000', 10),
    debug: process.env.TOPGUN_DEBUG === 'true',
    help: false,
    version: false,
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];

    switch (arg) {
      case '--url':
      case '-u':
        options.url = args[++i];
        break;

      case '--token':
      case '-t':
        options.token = args[++i];
        break;

      case '--maps':
      case '-m':
        options.maps = args[++i].split(',').map((m) => m.trim());
        break;

      case '--no-mutations':
        options.mutations = false;
        break;

      case '--no-subscriptions':
        options.subscriptions = false;
        break;

      case '--http':
        options.http = true;
        break;

      case '--port':
      case '-p':
        options.port = parseInt(args[++i], 10);
        break;

      case '--debug':
      case '-d':
        options.debug = true;
        break;

      case '--help':
      case '-h':
        options.help = true;
        break;

      case '--version':
      case '-v':
        options.version = true;
        break;
    }
  }

  return options;
}

function showHelp(): void {
  console.log(`
TopGun MCP Server v${VERSION}

Enables AI assistants (Claude, Cursor) to interact with TopGun databases.

USAGE:
  topgun-mcp [options]

OPTIONS:
  --url, -u <url>         TopGun server WebSocket URL
                          Default: ws://localhost:8080
                          Env: TOPGUN_URL

  --token, -t <token>     Authentication token for TopGun server
                          Env: TOPGUN_TOKEN

  --maps, -m <maps>       Comma-separated list of allowed maps
                          Default: all maps allowed
                          Env: TOPGUN_MAPS

  --no-mutations          Disable mutation operations (set, remove)

  --no-subscriptions      Disable subscription operations

  --http                  Start HTTP transport instead of stdio
                          Use this for web-based MCP clients

  --port, -p <port>       HTTP server port (only with --http)
                          Default: 3000
                          Env: TOPGUN_MCP_PORT

  --debug, -d             Enable debug logging to stderr
                          Env: TOPGUN_DEBUG=true

  --help, -h              Show this help message

  --version, -v           Show version number

EXAMPLES:

  # Start with stdio transport (for Claude Desktop / Cursor)
  topgun-mcp --url ws://localhost:8080

  # Start with specific maps and auth token
  topgun-mcp --url ws://prod.example.com:8080 --token <jwt> --maps tasks,users

  # Start HTTP server for web clients
  topgun-mcp --http --port 4000

  # Read-only mode (no mutations)
  topgun-mcp --no-mutations

CLAUDE DESKTOP CONFIGURATION:

Add to ~/Library/Application Support/Claude/claude_desktop_config.json:

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

CURSOR CONFIGURATION:

Add to .cursor/config.json in your workspace:

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

For more information, visit: https://github.com/topgunbuild/topgun
`);
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2));

  if (options.version) {
    console.log(VERSION);
    process.exit(0);
  }

  if (options.help) {
    showHelp();
    process.exit(0);
  }

  // Build config
  const config: MCPServerConfig = {
    topgunUrl: options.url,
    authToken: options.token,
    allowedMaps: options.maps,
    enableMutations: options.mutations,
    enableSubscriptions: options.subscriptions,
    debug: options.debug,
  };

  // Create logger
  const logger = createLogger({ debug: options.debug, name: 'topgun-mcp-cli' });

  logger.debug({ config }, 'Starting with config');

  // Create server
  const server = new TopGunMCPServer(config);

  // Handle graceful shutdown
  const shutdown = async () => {
    logger.info('Shutting down...');
    await server.stop();
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  try {
    if (options.http) {
      // Start HTTP transport
      const httpTransport = new HTTPTransport({
        port: options.port,
        debug: options.debug,
      });

      await httpTransport.start(server);

      logger.info({ port: options.port }, 'TopGun MCP Server (HTTP) listening');
      logger.info({ health: `http://localhost:${options.port}/health`, mcp: `http://localhost:${options.port}/mcp` }, 'Endpoints');
    } else {
      // Start stdio transport (default)
      await server.start();

      // Log to stderr so it doesn't interfere with MCP protocol on stdout
      logger.info('TopGun MCP Server started on stdio');
    }
  } catch (error) {
    logger.fatal({ error }, 'Failed to start TopGun MCP Server');
    process.exit(1);
  }
}

// Run if this is the main module
main().catch((error) => {
  // Create a simple logger for fatal errors (no debug mode here)
  const logger = createLogger({ name: 'topgun-mcp-cli' });
  logger.fatal({ error }, 'Fatal error');
  process.exit(1);
});
