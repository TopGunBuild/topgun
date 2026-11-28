import { decodeJwtExp } from '../utils/jwtExp';

const part = (value: unknown) => Buffer.from(JSON.stringify(value)).toString('base64url');
const jwt = (claims: unknown) => `${part({ alg: 'HS256', typ: 'JWT' })}.${part(claims)}.sig`;

describe('decodeJwtExp', () => {
  test('returns the exp of a JWT that carries one', () => {
    expect(decodeJwtExp(jwt({ sub: 'u1', exp: 1_800_000_000 }))).toBe(1_800_000_000);
    expect(decodeJwtExp(jwt({ exp: 0 }))).toBe(0);
  });

  test('returns null for a token with no numeric exp', () => {
    expect(decodeJwtExp(jwt({ sub: 'u1' }))).toBeNull();
    expect(decodeJwtExp(jwt({ exp: '1800000000' }))).toBeNull();
    expect(decodeJwtExp(jwt(null))).toBeNull();
    expect(decodeJwtExp(jwt(42))).toBeNull();
  });

  test('returns null for a string that is not a JWT', () => {
    expect(decodeJwtExp('')).toBeNull();
    expect(decodeJwtExp('opaque-token')).toBeNull();
    expect(decodeJwtExp('a.b')).toBeNull();
    expect(decodeJwtExp('a.b.c.d')).toBeNull();
  });

  test('returns null for a payload that is not base64url JSON', () => {
    expect(decodeJwtExp(`${part({})}.!!!.sig`)).toBeNull();
    expect(decodeJwtExp(`${part({})}.${part({ exp: 1 }).replace(/^./, '+')}.sig`)).toBeNull();
    expect(
      decodeJwtExp(`${part({})}.${Buffer.from('not json').toString('base64url')}.sig`),
    ).toBeNull();
    expect(decodeJwtExp(`${part({})}..sig`)).toBeNull();
  });
});
