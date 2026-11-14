/**
 * The one port rule of the MCP HTTP transport, used by the CLI for its string and by
 * HTTPTransport for its number: an integer from 1 to 65535. A string must be decimal digits
 * only, because parseInt would accept '3000abc' and Number would accept '3e3' or '0x10'.
 * Port 0 (an ephemeral port) is refused: the operator would not know where the server listens.
 *
 * @throws {Error} whose message starts with "must be", so a caller prefixes the setting's name.
 */
export function parsePort(value: string | number | undefined): number {
  const port =
    typeof value === 'number'
      ? value
      : value !== undefined && /^[0-9]+$/.test(value)
        ? Number(value)
        : NaN;
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    const got =
      value === undefined ? 'no value' : typeof value === 'number' ? String(value) : `'${value}'`;
    throw new Error(`must be an integer from 1 to 65535, got ${got}`);
  }
  return port;
}
