/**
 * Structured Logger for MCP Server
 *
 * Uses pino for JSON-structured logging.
 * Logs to stderr to not interfere with MCP protocol on stdout.
 */

import pino from 'pino';

/**
 * Create a logger instance for MCP server
 */
export function createLogger(options: { debug?: boolean; name?: string } = {}) {
  const { debug = false, name = 'topgun-mcp' } = options;

  // pino.destination uses SonicBoom (async buffered writes), which holds an open
  // write handle that prevents Node.js from exiting naturally in Jest. In test
  // environments, switch to synchronous mode so the handle is released immediately
  // after each write and Jest can exit cleanly without --forceExit.
  const isTest =
    typeof process !== 'undefined' &&
    (Boolean(process.env.JEST_WORKER_ID) || process.env.NODE_ENV === 'test');

  return pino(
    {
      name,
      level: debug ? 'debug' : 'info',
      // Always use stderr to not interfere with MCP stdio protocol
      transport: undefined,
    },
    pino.destination({ fd: 2, sync: isTest }),
  ); // fd 2 = stderr
}

/**
 * Default logger instance
 */
export const logger = createLogger();

/**
 * Logger type for type annotations
 */
export type Logger = ReturnType<typeof createLogger>;

/**
 * The fields of a server config that are safe to log, named one by one: a field added to the
 * config later is left out until someone decides it is safe, so a new credential (or a token in
 * a URL) is never logged by default. The TopGun URL is reduced to its scheme, host and port.
 */
export function configForLog(config: {
  topgunUrl?: string;
  authToken?: string;
  allowedMaps?: string[];
  enableMutations?: boolean;
  enableSubscriptions?: boolean;
  debug?: boolean;
}): Record<string, unknown> {
  let topgunOrigin: string | undefined;
  try {
    topgunOrigin = config.topgunUrl === undefined ? undefined : new URL(config.topgunUrl).origin;
  } catch {
    topgunOrigin = '[unparseable]';
  }
  return {
    topgunOrigin,
    authTokenSet: config.authToken !== undefined && config.authToken !== '',
    allowedMaps: config.allowedMaps,
    enableMutations: config.enableMutations,
    enableSubscriptions: config.enableSubscriptions,
    debug: config.debug,
  };
}
