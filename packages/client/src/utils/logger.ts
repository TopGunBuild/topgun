import pino from 'pino';

const isBrowser = typeof window !== 'undefined';

const logLevel = (typeof process !== 'undefined' && process.env && process.env.LOG_LEVEL) || 'info';

function createNodeLogger() {
  // Jest workers (JEST_WORKER_ID is auto-set by the Jest runner) keep the pino-pretty
  // thread-stream worker alive after all tests pass, preventing clean exit without --forceExit.
  // Fall through to plain stderr pino in tests; pretty-print stays active in dev/prod runtime.
  const isJestWorker = typeof process !== 'undefined' && Boolean(process.env.JEST_WORKER_ID);
  const wantPretty =
    typeof process !== 'undefined' && process.env.NODE_ENV !== 'production' && !isJestWorker;

  if (wantPretty) {
    try {
      // pino-pretty is a devDependency — when this package is consumed from npm without dev deps
      // (e.g. `npx -y @topgunbuild/mcp-server`), pino() throws synchronously here because the
      // transport target can't be resolved. Catch and fall back to plain JSON on stderr.
      return pino({
        level: logLevel,
        transport: {
          target: 'pino-pretty',
          options: {
            colorize: true,
            translateTime: 'SYS:standard',
            ignore: 'pid,hostname',
            destination: 2,
          },
        },
      });
    } catch {
      // fall through to the plain-stderr logger below
    }
  }

  // stderr destination keeps stdout clean for stdio-protocol consumers (MCP JSON-RPC).
  return pino({ level: logLevel }, pino.destination(2));
}

export const logger = isBrowser
  ? pino({ level: logLevel, browser: { asObject: true } })
  : createNodeLogger();

export type Logger = typeof logger;
