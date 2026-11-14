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
 *   --host <ip>           HTTP bind address (default: 127.0.0.1)
 *   --allow-public-bind   Allow a non-loopback --host
 *   --debug               Enable debug logging
 *   --help                Show help
 *   --version             Show version
 */

import { createRequire } from 'node:module';
import { isIP } from 'node:net';
import { TopGunMCPServer } from './TopGunMCPServer';
import { HTTPTransport, HTTPTransportConfigError, isLoopbackHost } from './transport/http';
import { parsePort } from './port';
import type { MCPServerConfig } from './types';
import { configForLog, createLogger } from './logger';

const require = createRequire(import.meta.url);
const { version: VERSION } = require('../package.json') as { version: string };

interface CLIOptions {
  url: string;
  token?: string;
  maps?: string[];
  mutations: boolean;
  subscriptions: boolean;
  http: boolean;
  // Kept raw and parsed only for --http, so a bad TOPGUN_MCP_PORT cannot affect stdio.
  port: string | undefined;
  host: string;
  allowPublicBind: boolean;
  debug: boolean;
  help: boolean;
  version: boolean;
}

function parseArgs(args: string[]): CLIOptions {
  const options: CLIOptions = {
    url: process.env.TOPGUN_URL || 'ws://localhost:8080',
    token: process.env.TOPGUN_TOKEN,
    maps: process.env.TOPGUN_MAPS?.split(',').map((m) => m.trim()),
    mutations: true,
    subscriptions: true,
    http: false,
    port: process.env.TOPGUN_MCP_PORT ?? '3000',
    host: process.env.TOPGUN_MCP_HOST ?? '127.0.0.1',
    allowPublicBind: /^(1|true)$/i.test(process.env.TOPGUN_MCP_ALLOW_PUBLIC_BIND ?? ''),
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
        options.port = args[++i];
        break;

      case '--host':
        // A missing value becomes '' so the transport refuses it instead of silently using the default.
        options.host = args[++i] ?? '';
        break;

      case '--allow-public-bind':
        options.allowPublicBind = true;
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

  --host <ip>             HTTP bind address, an IP literal (only with --http)
                          Default: 127.0.0.1
                          Env: TOPGUN_MCP_HOST

  --allow-public-bind     Allow a non-loopback --host (only with --http)
                          Env: TOPGUN_MCP_ALLOW_PUBLIC_BIND=1

                          Browser origins: the CLI allows none, on purpose
                          (MCP clients are not browsers); a request with an
                          Origin header gets 403. To serve a browser, use the
                          library's HTTPTransport with its corsOrigins option.

  TOPGUN_MCP_HTTP_TOKEN   Required with --http (environment only): the secret
                          every HTTP request must send as
                          'Authorization: Bearer <token>'; at least 32
                          characters. Generate one with:
                          openssl rand -base64 32

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
  TOPGUN_MCP_HTTP_TOKEN=$(openssl rand -base64 32) topgun-mcp --http --port 4000

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

// The CLI's names for the transport settings, so a refusal names what the operator sets.
// corsOrigins has no CLI name: the CLI never sets it.
const CLI_FIELD_NAMES: Partial<Record<HTTPTransportConfigError['field'], string>> = {
  bearerToken: 'TOPGUN_MCP_HTTP_TOKEN',
  host: '--host / TOPGUN_MCP_HOST',
  port: '--port / TOPGUN_MCP_PORT',
};

function refuseToStart(reason: string): never {
  process.stderr.write(`topgun-mcp: refusing to start: ${reason}\n`);
  process.exit(1);
}

function createHTTPTransport(options: CLIOptions): { transport: HTTPTransport; port: number } {
  let port: number;
  try {
    port = parsePort(options.port);
  } catch (error) {
    refuseToStart(`${CLI_FIELD_NAMES.port} ${(error as Error).message}`);
  }

  // A hostname is left to the transport's "must be an IP address" refusal.
  const { host, allowPublicBind } = options;
  if (isIP(host) !== 0 && !isLoopbackHost(host) && !allowPublicBind) {
    refuseToStart(
      `--host ${host} is not a loopback address; pass --allow-public-bind (or TOPGUN_MCP_ALLOW_PUBLIC_BIND=1) to bind it`,
    );
  }

  try {
    const transport = new HTTPTransport({
      port,
      host,
      allowPublicBind,
      // Read only here, so stdio needs no new variable.
      bearerToken: process.env.TOPGUN_MCP_HTTP_TOKEN ?? '',
      debug: options.debug,
    });
    return { transport, port };
  } catch (error) {
    if (error instanceof HTTPTransportConfigError) {
      const name = CLI_FIELD_NAMES[error.field] ?? error.field;
      refuseToStart(name + error.message.slice(error.field.length));
    }
    throw error;
  }
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

  logger.debug({ config: configForLog(config) }, 'Starting with config');

  // Validated before the server is built, so a refused start never creates a TopGun client
  // holding the auth token.
  const http = options.http ? createHTTPTransport(options) : undefined;

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
    if (http) {
      await http.transport.start(server);

      const { host } = options;
      const { port } = http;
      if (!isLoopbackHost(host)) {
        logger.warn(
          `MCP HTTP transport is bound to non-loopback ${host}:${port}: it is reachable from the network; every request needs the bearer token, and plain HTTP sends that token in clear — put TLS in front of it`,
        );
      }

      const base = isIP(host) === 6 ? `http://[${host}]:${port}` : `http://${host}:${port}`;
      logger.info({ host, port }, 'TopGun MCP Server (HTTP) listening');
      logger.info({ health: `${base}/health`, mcp: `${base}/mcp` }, 'Endpoints');
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
