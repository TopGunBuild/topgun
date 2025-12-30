/**
 * ResultSet exports
 */

export type { ResultSet } from './ResultSet';
export { SetResultSet } from './SetResultSet';
export { LazyResultSet, type IteratorFactory } from './LazyResultSet';
export { IntersectionResultSet } from './IntersectionResultSet';
export { UnionResultSet } from './UnionResultSet';
export { FilteringResultSet, type PredicateFn } from './FilteringResultSet';
export { SortedResultSet, createFieldComparator, type CompareFn } from './SortedResultSet';
export { LimitResultSet } from './LimitResultSet';
