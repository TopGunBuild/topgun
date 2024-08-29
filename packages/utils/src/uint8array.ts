/**
 * Returns true if the two passed Uint8Arrays have the same content
 */
export function equalBytes(a: Uint8Array, b: Uint8Array): boolean
{
  if (a === b)
  {
    return true
  }

  if (a.byteLength !== b.byteLength)
  {
    return false
  }

  for (let i = 0; i < a.byteLength; i++)
  {
    if (a[i] !== b[i])
    {
      return false
    }
  }

  return true
}


const hasUint8Array = (typeof Uint8Array === 'function');
const toStr         = Object.prototype.toString;

function nativeClass(v: unknown): string
{
  return toStr.call(v);
}

/**
 * Returns true if value is instance of Uint8Array
 */
export function isUint8Array(value: unknown): value is Uint8Array
{
  return (
    (hasUint8Array && value instanceof Uint8Array) || nativeClass(value) === '[object Uint8Array]'
  );
}

export function persistUint8Array(value: Uint8Array|number[]): Uint8Array
{
  if (isUint8Array(value)) {
    return value;
  }

  return new Uint8Array(value);
}
