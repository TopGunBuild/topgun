/**
 * Parses the CLI's HTTP port strictly: the whole value must be decimal digits and the number
 * must be in 1-65535. parseInt would accept '3000abc' and yield NaN for 'abc', and a NaN or
 * out-of-range port must refuse the start instead of binding something unexpected.
 */
export function parsePort(value: string | undefined): number {
  const port = value !== undefined && /^[0-9]+$/.test(value) ? Number(value) : NaN;
  if (!(port >= 1 && port <= 65535)) {
    const got = value === undefined ? 'no value' : `'${value}'`;
    throw new Error(`--port / TOPGUN_MCP_PORT must be an integer from 1 to 65535, got ${got}`);
  }
  return port;
}
