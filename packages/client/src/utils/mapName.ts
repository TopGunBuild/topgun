/**
 * Client persistence stores every record under the flat key `${mapName}:${key}`
 * and restores a map by `fullKey.startsWith(`${mapName}:`)`. That scheme is only
 * injective while no map name contains a `:`: if one name is a colon-prefix of
 * another (`"foo"` vs `"foo:bar"`), restoring `"foo"` also matches
 * `"foo:bar:someKey"` and merges a sibling map's records into the wrong map.
 * We forbid `:` in persisted map NAMES so the key scheme stays injective for all
 * new data. Map KEYS (and topics) are a separate keyspace and are NOT restricted.
 *
 * An empty name is rejected too: it has no injective key prefix and is not a
 * valid map identity.
 */

/**
 * The server stores a map under its name and refuses, before it acknowledges
 * anything, a name it cannot hold. The four clauses below are that rule, in the
 * server's order, so the client can say no at the call site instead of sending
 * a write that is certain to come back refused. The server puts no restriction
 * on which characters a name uses — `-`, `/`, `.` and `$` are all fine there.
 */
const MAX_MAP_NAME_BYTES = 512;
const RESERVED_BACKUP_SUFFIX = '__backup';

export type MapNameViolation = 'empty' | 'reserved_backup_suffix' | 'contains_nul' | 'too_long';

/** The length the server measures: UTF-8 bytes, not UTF-16 code units. */
function utf8ByteLength(name: string): number {
  return new TextEncoder().encode(name).length;
}

/**
 * Which clause of the server's name rule `name` breaks, or `null` when the
 * server would accept it. Never throws. A non-string is reported as `'empty'`:
 * the untyped overloads let a JS caller pass one, and it names no map.
 */
export function mapNameViolation(name: string): MapNameViolation | null {
  if (typeof name !== 'string' || name.length === 0) return 'empty';
  if (name.endsWith(RESERVED_BACKUP_SUFFIX)) return 'reserved_backup_suffix';
  if (name.includes('\0')) return 'contains_nul';
  if (utf8ByteLength(name) > MAX_MAP_NAME_BYTES) return 'too_long';
  return null;
}

/** True when the server would accept `name`. Says nothing about `:`. */
export function isStorableMapName(name: string): boolean {
  return mapNameViolation(name) === null;
}

/**
 * The write-time rule — never throws. A name is valid for `getMap` / `getORMap`
 * when the server would accept it AND it holds no `:` (the client's own
 * restriction, see the top of this file).
 *
 * Used ONLY by {@link assertValidMapName}. It is not the oplog-restore filter:
 * that is {@link isRestorableOpMapName}, a different set on purpose.
 */
export function isValidMapName(name: string): boolean {
  return isStorableMapName(name) && !name.includes(':');
}

/**
 * Which pending operations a restart puts back into the op log, by map name.
 *
 * Narrower than the write-time rule, and kept apart from it so that tightening
 * one never moves the other: an operation under a name only the server refuses
 * is restored and sent, because the server's refusal is the one thing that
 * retires it visibly. Only a name the client itself cannot store under — empty,
 * or holding `:` — is held back.
 */
export function isRestorableOpMapName(name: unknown): name is string {
  return typeof name === 'string' && name.length > 0 && !name.includes(':');
}

function describeMapNameViolation(name: string, violation: MapNameViolation): string {
  switch (violation) {
    case 'empty':
      return 'Invalid map name: name must not be empty and must be a string. A map name is its durable storage-key prefix and cannot be blank.';
    case 'reserved_backup_suffix':
      return `Invalid map name "${name}": a map name may not end in "${RESERVED_BACKUP_SUFFIX}" — that suffix is reserved by the server. Use it anywhere but the end of the name, or choose another word.`;
    case 'contains_nul':
      return 'Invalid map name: a map name may not contain the NUL character (U+0000). The server refuses such a name.';
    case 'too_long':
      return `Invalid map name: a map name may be at most ${MAX_MAP_NAME_BYTES} bytes of UTF-8, and this one is ${utf8ByteLength(name)} bytes. The server refuses a longer name.`;
  }
}

/**
 * Throwing guard for the entry points that only READ under a name (`query`,
 * `queryOnce`, `queryOncePaged`): the server's rule and nothing more. It does
 * not forbid `:` — a query writes no local key under its name, and it is how a
 * map that already holds such a name on the server can still be read.
 */
export function assertStorableMapName(name: string): void {
  const violation = mapNameViolation(name);
  if (violation !== null) {
    throw new Error(describeMapNameViolation(name, violation));
  }
}

/**
 * Throwing guard for the map-creation boundary (`getMap`/`getORMap`): the
 * server's rule plus the client's `:` restriction, one distinct message per
 * clause. Must NOT be called inside restore/replay loops — those hold back
 * entries via {@link isRestorableOpMapName} instead of aborting.
 */
export function assertValidMapName(name: string): void {
  if (isValidMapName(name)) return;
  // The server's clauses first, in the server's order; ":" is the client's own
  // and is reported only for a name the server would otherwise take.
  assertStorableMapName(name);
  throw new Error(
    `Invalid map name "${name}": the ":" character is not allowed in map names because it is the storage key-scheme separator (records persist under "\${mapName}:\${key}"). Use "-", "/", or "_" instead (e.g. "${name.replace(
      /:/g,
      '-',
    )}"). Note: ":" is still allowed in map KEYS and in topic names.`,
  );
}

/**
 * Longest-held-name restore discriminator (shared by both OR-Map restore seams:
 * `TopGunClient.restoreORMap` and `SyncEngine.instantiateAndRestoreOrMap`).
 *
 * The flat `${mapName}:${key}` scheme is not injective for legacy colon-named
 * stores, so a key matched by prefix `${mapName}:` may actually belong to a
 * LONGER held name. When restoring map `mapName`, a matched key with remainder
 * `keyPart` is owned by a longer map iff some colon-prefix of `keyPart`, joined
 * back onto `mapName`, is itself a held name (e.g. remainder `b:k` under `a`
 * yields candidate `a:b` — if `a:b` is held, `a:b:k` belongs to `a:b`, not `a`).
 * The held-set is the discriminator; when it is `null` (before the first sync of
 * a connection) no longer name is known and nothing is skipped.
 */
export function keyBelongsToLongerHeldName(
  mapName: string,
  keyPart: string,
  held: Set<string> | null,
): boolean {
  if (!held) return false;
  let idx = keyPart.indexOf(':');
  while (idx !== -1) {
    const longer = `${mapName}:${keyPart.substring(0, idx)}`;
    if (held.has(longer)) return true;
    idx = keyPart.indexOf(':', idx + 1);
  }
  return false;
}
