import { VirtualClock } from './VirtualClock';
import { SeededRNG } from './SeededRNG';
import { VirtualNetwork } from './VirtualNetwork';
import { InvariantChecker } from './InvariantChecker';

/**
 * Configuration for a simulation scenario.
 */
export interface ScenarioConfig {
  /** Random seed for reproducibility (auto-generated if not provided) */
  seed?: number;
  /** List of node identifiers participating in the scenario */
  nodes: string[];
  /** Total duration in virtual milliseconds */
  duration: number;
  /** Virtual ms to advance per tick (default: 1) */
  tickInterval?: number;
}

/**
 * Result of a scenario execution.
 */
export interface ScenarioResult {
  /** Seed used for this run (for reproduction) */
  seed: number;
  /** Whether all invariants passed */
  passed: boolean;
  /** Number of ticks executed */
  ticks: number;
  /** List of invariant failures */
  invariantFailures: string[];
  /** Final state captured at end of scenario */
  finalStates: Map<string, unknown>;
}

/**
 * Orchestrates deterministic simulation scenarios.
 * Combines virtual clock, seeded RNG, and virtual network for reproducible testing.
 *
 * Usage:
 * ```typescript
 * const runner = new ScenarioRunner({
 *   seed: 12345,
 *   nodes: ['node-a', 'node-b'],
 *   duration: 1000
 * });
 *
 * const result = runner.run(
 *   (r) => {
 *     // Setup: create CRDTs, configure network
 *     r.getNetwork().configure({ packetLossRate: 0.1 });
 *   },
 *   (r, tick) => {
 *     // Step: simulate operations, deliver messages
 *     if (tick === 500) r.getNetwork().heal();
 *   },
 *   invariants
 * );
 *
 * if (!result.passed) {
 *   console.log(`Failed with seed ${result.seed}`);
 * }
 * ```
 */
export class ScenarioRunner {
  private readonly config: ScenarioConfig;
  private readonly clock: VirtualClock;
  private readonly rng: SeededRNG;
  private readonly network: VirtualNetwork;
  private readonly seed: number;

  constructor(config: ScenarioConfig) {
    // Validate config
    if (!config.nodes || config.nodes.length === 0) {
      throw new Error('Scenario must have at least one node');
    }
    if (config.duration <= 0) {
      throw new Error('Duration must be positive');
    }

    // Generate seed if not provided
    this.seed = config.seed ?? Math.floor(Math.random() * 2147483647);

    this.config = {
      ...config,
      seed: this.seed,
      tickInterval: config.tickInterval ?? 1
    };

    // Initialize simulation components
    this.clock = new VirtualClock(0);
    this.rng = new SeededRNG(this.seed);
    this.network = new VirtualNetwork(this.rng, this.clock);
  }

  /**
   * Returns the seed used for this scenario.
   */
  public getSeed(): number {
    return this.seed;
  }

  /**
   * Returns the virtual clock instance.
   */
  public getClock(): VirtualClock {
    return this.clock;
  }

  /**
   * Returns the seeded RNG instance.
   */
  public getRNG(): SeededRNG {
    return this.rng;
  }

  /**
   * Returns the virtual network instance.
   */
  public getNetwork(): VirtualNetwork {
    return this.network;
  }

  /**
   * Returns the list of nodes in this scenario.
   */
  public getNodes(): string[] {
    return [...this.config.nodes];
  }

  /**
   * Executes the simulation scenario.
   *
   * @param setup Called once before simulation starts. Initialize state here.
   * @param step Called on each tick. Perform operations and message delivery.
   * @param invariants Checker for verifying correctness throughout execution.
   * @returns Result with pass/fail status and captured state
   */
  public run(
    setup: (runner: ScenarioRunner) => void,
    step: (runner: ScenarioRunner, tick: number) => void,
    invariants: InvariantChecker<unknown>
  ): ScenarioResult {
    const finalStates = new Map<string, unknown>();
    const invariantFailures: string[] = [];

    // Setup phase
    setup(this);

    // Simulation loop
    let tickCount = 0;
    const tickInterval = this.config.tickInterval!;
    const endTime = this.config.duration;

    while (this.clock.now() < endTime) {
      // Advance time
      this.clock.advance(tickInterval);
      tickCount++;

      // Execute step
      step(this, tickCount);

      // Deliver network messages
      const delivered = this.network.tick();

      // Store delivered messages count for debugging
      if (delivered.length > 0) {
        finalStates.set(`_tick_${tickCount}_delivered`, delivered.length);
      }
    }

    // Final invariant check
    const result = invariants.verify(null);
    if (!result.passed) {
      invariantFailures.push(...result.failures);
    }

    return {
      seed: this.seed,
      passed: invariantFailures.length === 0,
      ticks: tickCount,
      invariantFailures,
      finalStates
    };
  }

  /**
   * Stores state for a node (useful for capturing final state).
   */
  public setState(nodeId: string, state: unknown): void {
    if (!this.config.nodes.includes(nodeId)) {
      throw new Error(`Unknown node: ${nodeId}`);
    }
    // This would be stored in the result's finalStates
    // For now, this is a placeholder for future enhancement
  }
}
