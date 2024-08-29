import { Digest, PageDigest, ValueDigest } from '../src'

describe('Digest', () =>
{
  it('asBytes() should return the original bytes', () =>
  {
    const b = new Uint8Array([42, 42, 42, 42]);
    const d = new Digest<4>(b);

    expect(b.every((value, index) => value === d.asBytes()[index])).toBeTruthy();
  });

  test('base64 format', () =>
  {
    const d = new Digest(new Uint8Array([0x62, 0x61, 0x6e, 0x61, 0x6e, 0x61, 0x73, 0x0a]), 8);
    expect(d.toString()).toBe('YmFuYW5hcwo=')

    const value = new ValueDigest(
      new Digest(new Uint8Array([0x62, 0x61, 0x6e, 0x61, 0x6e, 0x61, 0x73, 0x0a]), 8),
    )
    expect(value.toString()).toBe('YmFuYW5hcwo=')

    const page = PageDigest.from(
      new Digest(new Uint8Array([
        0x62, 0x61, 0x6e, 0x61, 0x6e, 0x61, 0x73, 0x0a, 0x62, 0x61, 0x6e,
        0x61, 0x6e, 0x61, 0x73, 0x0a,
      ]), 16),
    );
    expect(page.toString()).toBe('YmFuYW5hcwpiYW5hbmFzCg==')
  })

  test('as bytes', () =>
  {
    const b      = [42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42, 42];
    const d      = PageDigest.from(
      new Digest(new Uint8Array(b), 16),
    );
    const result = Array.from(d.valueOf().asBytes());
    expect(result).toEqual(b)
  })
})
