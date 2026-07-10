import { isValidMapName, assertValidMapName, keyBelongsToLongerHeldName } from '../utils/mapName';

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
