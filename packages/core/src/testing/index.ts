/**
 * Deterministic Simulation Testing (DST) Infrastructure
 *
 * Provides tools for reproducible distributed systems testing:
 * - VirtualClock: Injectable time source for deterministic time control
 * - SeededRNG: Reproducible randomness for chaos injection
 * - VirtualNetwork: Simulated network with packet loss, latency, partitions
 * - InvariantChecker: Property-based assertions for CRDT consistency
 * - ScenarioRunner: Test orchestration with seed-based reproducibility
 */

export { VirtualClock, RealClock } from './VirtualClock';
export type { ClockSource } from './VirtualClock';

export { SeededRNG } from './SeededRNG';

export { VirtualNetwork } from './VirtualNetwork';
export type { NetworkConfig, Message } from './VirtualNetwork';

export { InvariantChecker, CRDTInvariants } from './InvariantChecker';
export type { Invariant, InvariantResult } from './InvariantChecker';

export { ScenarioRunner } from './ScenarioRunner';
export type { ScenarioConfig, ScenarioResult } from './ScenarioRunner';
