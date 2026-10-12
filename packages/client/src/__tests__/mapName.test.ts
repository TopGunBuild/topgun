import {
  assertStorableMapName,
  assertValidMapName,
  isRestorableOpMapName,
  isStorableMapName,
  isValidMapName,
  keyBelongsToLongerHeldName,
  mapNameViolation,
} from '../utils/mapName';

const ASCII_512 = 'a'.repeat(512);
const ASCII_513 = 'a'.repeat(513);
// 300 characters of two UTF-8 bytes each: 300 by `length`, 600 by the server's count.
const TWO_BYTE_300 = 'é'.repeat(300);

describe('mapName predicates', () => {
  describe('isValidMapName', () => {
    it.each([
      // colon-free names are valid
      ['users', true],
      ['a-b', true],
      ['a/b', true],
      ['a_b', true],
      ['doc-room-1', true],
      ['post123', true],
      ['__sys__', true],
      // any colon anywhere is invalid
      ['a:b', false],
      [':leading', false],
      ['trailing:', false],
      ['a:b:c', false],
      // empty string is invalid (no injective key prefix, no map identity)
      ['', false],
    ])('isValidMapName(%j) === %s', (name, expected) => {
      expect(isValidMapName(name as string)).toBe(expected);
    });
  });

  describe('isValidMapName — the write-time rule (server rule plus no ":")', () => {
    it.each([
      ['empty', ''],
      ['a ":"', 'a:b'],
      ['the reserved suffix', 'x__backup'],
      ['a NUL character', 'a\0b'],
      ['513 bytes of ASCII', ASCII_513],
      ['300 two-byte characters (600 bytes)', TWO_BYTE_300],
    ])('is false for %s', (_label, name) => {
      expect(isValidMapName(name)).toBe(false);
    });

    it.each([[undefined], [null], [42], [{}]])('is false for the non-string %p', (name) => {
      expect(isValidMapName(name as unknown as string)).toBe(false);
    });

    it.each([
      ['a-b'],
      ['a/b'],
      ['a.b'],
      ['$sys/x'],
      // The suffix is reserved only at the end of a name.
      ['__backup_data'],
      [ASCII_512],
    ])('is true for %j', (name) => {
      expect(isValidMapName(name)).toBe(true);
    });
  });

  describe('mapNameViolation — the server rule, in the server order', () => {
    it.each([
      ['', 'empty'],
      ['x__backup', 'reserved_backup_suffix'],
      ['a\0b', 'contains_nul'],
      [ASCII_513, 'too_long'],
      [TWO_BYTE_300, 'too_long'],
      // Earlier clauses win over later ones.
      ['a\0b__backup', 'reserved_backup_suffix'],
      [`\0${ASCII_513}`, 'contains_nul'],
    ])('mapNameViolation(%j) === %j', (name, expected) => {
      expect(mapNameViolation(name)).toBe(expected);
      expect(isStorableMapName(name)).toBe(false);
    });

    it.each([['a:b'], ['a-b'], ['users/profiles'], [ASCII_512]])(
      'accepts %j — the server has no rule about ":"',
      (name) => {
        expect(mapNameViolation(name)).toBeNull();
        expect(isStorableMapName(name)).toBe(true);
      },
    );
  });

  describe('the 512-byte bound is measured in UTF-8 bytes, for every kind of character', () => {
    // What the server counts. Available in the test runtime; the rule itself
    // must not depend on it (see the next block).
    const utf8Bytes = (name: string): number => new TextEncoder().encode(name).length;

    const units: Array<[string, string]> = [
      ['ASCII (1 byte)', 'a'],
      ['a two-byte character', 'é'],
      ['a three-byte character', '€'],
      ['an emoji — a surrogate pair, 4 bytes', '😀'],
      // An unpaired surrogate is encoded as U+FFFD, 3 bytes.
      ['a lone high surrogate', '\uD83D'],
      ['a lone low surrogate', '\uDE00'],
      ['a low surrogate followed by a high one — two lone surrogates, not a pair', '\uDE00\uD83D'],
      ['a mix of all of them', 'aé€😀\uD83Dz\uDE00'],
    ];
    // Repeat counts on both sides of the bound for every unit width above.
    const repeats = [1, 28, 29, 56, 57, 85, 86, 100, 128, 129, 170, 171, 256, 257, 512, 513];

    it.each(units)('agrees with the encoded length for %s', (_label, unit) => {
      for (const n of repeats) {
        const name = unit.repeat(n);
        const expected = utf8Bytes(name) > 512 ? 'too_long' : null;
        expect([n, mapNameViolation(name)]).toEqual([n, expected]);
      }
    });

    it.each(units)('reports the encoded length in the message for %s', (_label, unit) => {
      const name = unit.repeat(513);
      expect(() => assertStorableMapName(name)).toThrow(`${utf8Bytes(name)} bytes`);
    });
  });

  describe('the rule needs no TextEncoder global', () => {
    // Not every JavaScript runtime an app ships to has one. A missing global
    // must never turn "open a map" into a ReferenceError.
    const original = globalThis.TextEncoder;

    beforeEach(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- removing a global for the duration of the test
      delete (globalThis as any).TextEncoder;
    });

    afterEach(() => {
      globalThis.TextEncoder = original;
    });

    it('accepts an ordinary name', () => {
      expect(typeof globalThis.TextEncoder).toBe('undefined');
      expect(mapNameViolation('users')).toBeNull();
      expect(() => assertValidMapName('users')).not.toThrow();
    });

    it('accepts a 512-byte name and refuses a 513-byte one', () => {
      expect(mapNameViolation(ASCII_512)).toBeNull();
      expect(mapNameViolation(ASCII_513)).toBe('too_long');
    });

    it('refuses 300 two-byte characters (600 bytes) and says so', () => {
      expect(mapNameViolation(TWO_BYTE_300)).toBe('too_long');
      expect(() => assertValidMapName(TWO_BYTE_300)).toThrow(/600 bytes/);
    });
  });

  describe('isRestorableOpMapName — the oplog-restore filter, a different set', () => {
    it.each([['x__backup'], ['a\0b'], [ASCII_513], ['users']])(
      'passes %j: a name only the server refuses is restored and sent',
      (name) => {
        expect(isRestorableOpMapName(name)).toBe(true);
      },
    );

    it.each([[''], ['a:b'], [undefined], [7]])('holds back %p', (name) => {
      expect(isRestorableOpMapName(name)).toBe(false);
    });
  });

  describe('assertStorableMapName', () => {
    it('does not throw for a name with ":"', () => {
      expect(() => assertStorableMapName('a:b')).not.toThrow();
    });

    it('never mentions ":" — that restriction is not the server\'s', () => {
      for (const name of ['', 'x__backup', 'a\0b', ASCII_513]) {
        expect(() => assertStorableMapName(name)).toThrow(Error);
        expect(() => assertStorableMapName(name)).not.toThrow(/":"/);
      }
    });
  });

  describe('assertValidMapName', () => {
    it('does not throw for a colon-free name', () => {
      expect(() => assertValidMapName('a-b')).not.toThrow();
      expect(() => assertValidMapName('users')).not.toThrow();
    });

    it('throws an Error identifying ":" as the offending character', () => {
      expect(() => assertValidMapName('a:b')).toThrow(Error);
      // Message names the ":" separator and suggests a colon-free replacement.
      expect(() => assertValidMapName('a:b')).toThrow(/":"/);
      expect(() => assertValidMapName('a:b')).toThrow(/not allowed in map names/);
      expect(() => assertValidMapName('a:b')).toThrow(/a-b/);
    });

    it('throws a distinct, empty-specific message for the empty name', () => {
      expect(() => assertValidMapName('')).toThrow(Error);
      expect(() => assertValidMapName('')).toThrow(/must not be empty/);
      // The empty-name path must NOT reuse the colon message.
      expect(() => assertValidMapName('')).not.toThrow(/":" character is not allowed/);
    });

    it('says the "__backup" suffix is reserved, and does not blame ":"', () => {
      expect(() => assertValidMapName('x__backup')).toThrow(/__backup/);
      expect(() => assertValidMapName('x__backup')).toThrow(/reserved by the server/);
      expect(() => assertValidMapName('x__backup')).not.toThrow(/":"/);
    });

    it('states the 512-byte bound and the actual byte length', () => {
      expect(() => assertValidMapName(ASCII_513)).toThrow(/512 bytes/);
      expect(() => assertValidMapName(ASCII_513)).toThrow(/513 bytes/);
      // Bytes, not characters: 300 characters, 600 bytes.
      expect(() => assertValidMapName(TWO_BYTE_300)).toThrow(/600 bytes/);
    });

    it('has a message of its own for a NUL character', () => {
      expect(() => assertValidMapName('a\0b')).toThrow(/NUL/);
    });

    it('still suggests "-", "/" or "_" in place of ":"', () => {
      expect(() => assertValidMapName('a:b')).toThrow(/Use "-", "\/", or "_" instead/);
    });
  });

  describe('keyBelongsToLongerHeldName (longest-held-name discriminator)', () => {
    it('returns false when the held-set is null (no snapshot yet)', () => {
      expect(keyBelongsToLongerHeldName('a', 'b:k', null)).toBe(false);
    });

    it('returns false when no longer held name owns the key', () => {
      expect(keyBelongsToLongerHeldName('a', 'b:k', new Set(['a']))).toBe(false);
    });

    it('returns true when a longer held name owns the key', () => {
      // remainder "b:k" under "a" → candidate "a:b"; held → belongs to the longer map
      expect(keyBelongsToLongerHeldName('a', 'b:k', new Set(['a', 'a:b']))).toBe(true);
    });

    it('tests EVERY colon-prefix of the remainder', () => {
      // remainder "x:y:k" under "a" → candidates "a:x" and "a:x:y"
      expect(keyBelongsToLongerHeldName('a', 'x:y:k', new Set(['a:x:y']))).toBe(true);
      expect(keyBelongsToLongerHeldName('a', 'x:y:k', new Set(['a:x']))).toBe(true);
      expect(keyBelongsToLongerHeldName('a', 'x:y:k', new Set(['a:z']))).toBe(false);
    });

    it('a remainder with no colon can never belong to a longer name', () => {
      // The ordinary composite-KEY case: real map "tags", key "post123" → no phantom owner.
      expect(keyBelongsToLongerHeldName('tags', 'post123', new Set(['tags', 'tags:post']))).toBe(
        false,
      );
    });
  });
});
