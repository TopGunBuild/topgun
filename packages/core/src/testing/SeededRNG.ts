/**
 * Seeded pseudo-random number generator for deterministic testing.
 * Uses the mulberry32 algorithm for fast, high-quality randomness.
 *
 * Usage:
 * ```typescript
 * const rng = new SeededRNG(12345);
 * rng.random(); // 0.6011..., always same for seed 12345
 * rng.randomInt(1, 10); // Deterministic integer in range
 * ```
 */
export class SeededRNG {
  private state: number;
  private readonly originalSeed: number;

  /**
   * @param seed Integer seed value. Same seed = same sequence.
   */
  constructor(seed: number) {
    if (!Number.isInteger(seed)) {
      throw new Error('Seed must be an integer');
    }
    this.state = seed >>> 0; // Ensure unsigned 32-bit
    this.originalSeed = this.state;
  }

  /**
   * Returns the original seed used to construct this RNG.
   */
  public getSeed(): number {
    return this.originalSeed;
  }

  /**
   * Generates the next random number in [0, 1).
   * Uses mulberry32 algorithm for deterministic, high-quality randomness.
   */
  public random(): number {
    let t = this.state += 0x6D2B79F5;
    t = Math.imul(t ^ t >>> 15, t | 1);
    t ^= t + Math.imul(t ^ t >>> 7, t | 61);
    this.state = t;
    return ((t ^ t >>> 14) >>> 0) / 4294967296;
  }

  /**
   * Generates a random integer in [min, max] (inclusive).
   * @param min Minimum value (inclusive)
   * @param max Maximum value (inclusive)
   */
  public randomInt(min: number, max: number): number {
    if (!Number.isInteger(min) || !Number.isInteger(max)) {
      throw new Error('Min and max must be integers');
    }
    if (min > max) {
      throw new Error('Min must be less than or equal to max');
    }
    const range = max - min + 1;
    return Math.floor(this.random() * range) + min;
  }

  /**
   * Generates a random boolean value.
   * @param probability Probability of returning true (default: 0.5)
   */
  public randomBool(probability: number = 0.5): boolean {
    if (probability < 0 || probability > 1) {
      throw new Error('Probability must be between 0 and 1');
    }
    return this.random() < probability;
  }

  /**
   * Shuffles an array in place using Fisher-Yates algorithm.
   * Returns the shuffled array.
   * @param array Array to shuffle
   */
  public shuffle<T>(array: T[]): T[] {
    for (let i = array.length - 1; i > 0; i--) {
      const j = this.randomInt(0, i);
      [array[i], array[j]] = [array[j], array[i]];
    }
    return array;
  }

  /**
   * Picks a random element from an array.
   * @param array Array to pick from
   * @returns Random element, or undefined if array is empty
   */
  public pick<T>(array: T[]): T | undefined {
    if (array.length === 0) return undefined;
    return array[this.randomInt(0, array.length - 1)];
  }

  /**
   * Resets the RNG to its original seed.
   * Useful for reproducing a sequence from the start.
   */
  public reset(): void {
    this.state = this.originalSeed;
  }
}
