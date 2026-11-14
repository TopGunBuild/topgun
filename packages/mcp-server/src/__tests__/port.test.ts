import { parsePort } from '../port';

describe('parsePort', () => {
  it('accepts an integer from 1 to 65535', () => {
    expect(parsePort('3000')).toBe(3000);
    expect(parsePort('1')).toBe(1);
    expect(parsePort('65535')).toBe(65535);
  });

  it.each(['abc', '', '  ', '3000abc', '3e3', '0', '65536', '-1', '1.5', '0x10', ' 3000'])(
    'refuses %j',
    (value) => {
      expect(() => parsePort(value)).toThrow('must be an integer from 1 to 65535');
    },
  );

  it('applies the same rule to a number', () => {
    expect(parsePort(3000)).toBe(3000);
    for (const value of [0, -1, 65536, 1.5, NaN, Infinity]) {
      expect(() => parsePort(value)).toThrow('must be an integer from 1 to 65535');
    }
  });

  it('refuses a missing value', () => {
    expect(() => parsePort(undefined)).toThrow('got no value');
  });
});
