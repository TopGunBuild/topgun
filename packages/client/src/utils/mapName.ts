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
 * Boolean predicate — never throws. Returns `false` when `name` contains `:` or
 * is the empty string, `true` otherwise. Used by the throwing map-creation guard
 * AND by the non-throwing oplog-restore filter (which must shed, not throw).
 */
export function isValidMapName(name: string): boolean {
  if (name.length === 0) return false;
  if (name.includes(':')) return false;
  return true;
}

/**
 * Throwing guard for the map-creation boundary (`getMap`/`getORMap`). Rejects a
 * name that fails `isValidMapName` with a clear, actionable error. Must NOT be
 * called inside restore/replay loops — those shed invalid entries via
 * `isValidMapName` directly instead of aborting.
 */
export function assertValidMapName(name: string): void {
  if (isValidMapName(name)) return;
  if (name.length === 0) {
    throw new Error(
      'Invalid map name: name must not be empty. A map name is its durable storage-key prefix and cannot be blank.',
    );
  }
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
