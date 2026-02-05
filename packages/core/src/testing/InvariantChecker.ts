import { LWWMap } from '../LWWMap';
import { ORMap } from '../ORMap';
import { Timestamp, HLC } from '../HLC';
import { MerkleTree } from '../MerkleTree';

/**
 * Invariant function signature.
 * Returns true if the invariant holds, false otherwise.
 */
export type Invariant<T> = (state: T) => boolean;

/**
 * Result of invariant verification.
 */
export interface InvariantResult {
  passed: boolean;
  failures: string[];
}

/**
 * Checker for property-based invariants.
 * Used to verify CRDT consistency during simulation.
 *
 * Usage:
 * ```typescript
 * const checker = new InvariantChecker<MyState>();
 * checker.addInvariant('no-nulls', (state) => state.value !== null);
 * const result = checker.verify(state);
 * if (!result.passed) {
 *   console.log('Failures:', result.failures);
 * }
 * ```
 */
export class InvariantChecker<T> {
  private invariants: Map<string, Invariant<T>> = new Map();

  /**
   * Adds an invariant to be checked.
   * @param name Unique name for this invariant
   * @param check Function that returns true if invariant holds
   */
  public addInvariant(name: string, check: Invariant<T>): void {
    if (this.invariants.has(name)) {
      throw new Error(`Invariant '${name}' already exists`);
    }
    this.invariants.set(name, check);
  }

  /**
   * Removes an invariant by name.
   */
  public removeInvariant(name: string): boolean {
    return this.invariants.delete(name);
  }

  /**
   * Verifies all invariants against the provided state.
   * @returns Result with pass/fail status and list of failed invariants
   */
  public verify(state: T): InvariantResult {
    const failures: string[] = [];

    for (const [name, check] of this.invariants.entries()) {
      try {
        if (!check(state)) {
          failures.push(name);
        }
      } catch (error) {
        failures.push(`${name} (exception: ${error instanceof Error ? error.message : String(error)})`);
      }
    }

    return {
      passed: failures.length === 0,
      failures
    };
  }

  /**
   * Returns the number of registered invariants.
   */
  public get count(): number {
    return this.invariants.size;
  }

  /**
   * Clears all invariants.
   */
  public clear(): void {
    this.invariants.clear();
  }
}

/**
 * Predefined CRDT invariants for common testing scenarios.
 */
export const CRDTInvariants = {
  /**
   * Verifies LWW-Map convergence: all maps contain the same values for same keys.
   */
  lwwConvergence: <K, V>(maps: LWWMap<K, V>[]): boolean => {
    if (maps.length < 2) return true;

    const reference = maps[0];
    const refKeys = new Set(reference.allKeys());

    for (let i = 1; i < maps.length; i++) {
      const other = maps[i];
      const otherKeys = new Set(other.allKeys());

      // Check same key set
      if (refKeys.size !== otherKeys.size) return false;
      for (const key of refKeys) {
        if (!otherKeys.has(key)) return false;
      }

      // Check same values for each key
      for (const key of refKeys) {
        const refRecord = reference.getRecord(key);
        const otherRecord = other.getRecord(key);

        if (!refRecord || !otherRecord) {
          if (refRecord !== otherRecord) return false;
          continue;
        }

        // Compare values (tombstones are both null)
        if (refRecord.value !== otherRecord.value) return false;

        // Compare timestamps
        if (HLC.compare(refRecord.timestamp, otherRecord.timestamp) !== 0) {
          return false;
        }
      }
    }

    return true;
  },

  /**
   * Verifies OR-Map convergence: all maps contain the same values for same keys.
   */
  orMapConvergence: <K, V>(maps: ORMap<K, V>[]): boolean => {
    if (maps.length < 2) return true;

    const reference = maps[0];
    const refKeys = reference.allKeys();

    for (let i = 1; i < maps.length; i++) {
      const other = maps[i];
      const otherKeys = new Set(other.allKeys());

      // Check same key set
      if (refKeys.length !== otherKeys.size) return false;
      for (const key of refKeys) {
        if (!otherKeys.has(key)) return false;
      }

      // Check same records for each key
      for (const key of refKeys) {
        const refRecords = reference.getRecords(key);
        const otherRecords = other.getRecords(key);

        if (refRecords.length !== otherRecords.length) return false;

        // Sort by tag for comparison
        const refSorted = [...refRecords].sort((a, b) => a.tag.localeCompare(b.tag));
        const otherSorted = [...otherRecords].sort((a, b) => a.tag.localeCompare(b.tag));

        for (let j = 0; j < refSorted.length; j++) {
          if (refSorted[j].tag !== otherSorted[j].tag) return false;
          if (refSorted[j].value !== otherSorted[j].value) return false;
          if (HLC.compare(refSorted[j].timestamp, otherSorted[j].timestamp) !== 0) {
            return false;
          }
        }
      }

      // Check same tombstones
      const refTombstones = new Set(reference.getTombstones());
      const otherTombstones = new Set(other.getTombstones());

      if (refTombstones.size !== otherTombstones.size) return false;
      for (const tag of refTombstones) {
        if (!otherTombstones.has(tag)) return false;
      }
    }

    return true;
  },

  /**
   * Verifies HLC monotonicity: timestamps are strictly increasing.
   */
  hlcMonotonicity: (timestamps: Timestamp[]): boolean => {
    for (let i = 1; i < timestamps.length; i++) {
      if (HLC.compare(timestamps[i - 1], timestamps[i]) >= 0) {
        return false;
      }
    }
    return true;
  },

  /**
   * Verifies Merkle tree consistency: trees with same data have same root hash.
   */
  merkleConsistency: (trees: MerkleTree[]): boolean => {
    if (trees.length < 2) return true;

    const referenceHash = trees[0].getRootHash();
    for (let i = 1; i < trees.length; i++) {
      if (trees[i].getRootHash() !== referenceHash) {
        return false;
      }
    }
    return true;
  }
};
