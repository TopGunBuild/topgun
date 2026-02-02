/**
 * Subscriptions module - Distributed subscription coordinators
 *
 * @module subscriptions
 */

// Base class and interfaces
export {
  DistributedSubscriptionBase,
  type DistributedSubscription,
  type DistributedSubscriptionConfig,
  type DistributedSubscriptionResult,
} from './DistributedSubscriptionBase';

// Type-specific coordinators
export { DistributedSearchCoordinator } from './DistributedSearchCoordinator';
export { DistributedQueryCoordinator } from './DistributedQueryCoordinator';

// Facade (main entry point for backward compatibility)
export { DistributedSubscriptionCoordinator } from './DistributedSubscriptionCoordinator';
