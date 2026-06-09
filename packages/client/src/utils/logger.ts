import pino from 'pino';

const isBrowser = typeof window !== 'undefined';

const logLevel = (typeof process !== 'undefined' && process.env && process.env.LOG_LEVEL) || 'info';

function createNodeLogger() {
  // pino-pretty uses thread-stream (a worker thread) that stays alive after all tests
  // pass, preventing clean Jest exit without --forceExit. Disable pretty-print in any
  // Jest environment — both --runInBand (NODE_ENV=test, no JEST_WORKER_ID) and worker
  // mode (JEST_WORKER_ID is set). Pretty-print stays active in dev/prod runtime.
  const isJestEnv =
    typeof process !== 'undefined' &&
    (Boolean(process.env.JEST_WORKER_ID) || process.env.NODE_ENV === 'test');
  const wantPretty =
    typeof process !== 'undefined' && process.env.NODE_ENV !== 'production' && !isJestEnv;

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

  // pino.destination uses SonicBoom (async buffered writes), which keeps an open
  // write handle that prevents Node.js from exiting cleanly in Jest without --forceExit.
  // In test environments, sync mode releases the handle immediately after each write.
  const isTest = Boolean(process.env.JEST_WORKER_ID) || process.env.NODE_ENV === 'test';

  // stderr destination keeps stdout clean for stdio-protocol consumers (MCP JSON-RPC).
  return pino({ level: logLevel }, pino.destination({ fd: 2, sync: isTest }));
}

export const logger = isBrowser
  ? pino({ level: logLevel, browser: { asObject: true } })
  : createNodeLogger();

export type Logger = typeof logger;
