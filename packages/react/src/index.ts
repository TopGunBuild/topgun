export * from './TopGunProvider';
export * from './hooks/useClient';
export * from './hooks/useQuery';
export * from './hooks/useMutation';
export * from './hooks/useMap';
export * from './hooks/useORMap';
export * from './hooks/useSyncState';
export * from './hooks/useTopic';

// Per-record sync-state type re-export from @topgunbuild/client
export type { RecordSyncState } from '@topgunbuild/client';
// Query result item type — the element type of `useQuery().data` (`T & { _key }`).
// Re-exported so apps can annotate component props with the hook's own item type.
export type { QueryResultItem } from '@topgunbuild/client';
export * from './hooks/usePNCounter';
export * from './hooks/useEventJournal';

// Merge rejection observer (built-in CRDT merge collisions, NOT custom
// resolvers — that surface is on the v2.x WASM-sandbox roadmap).
export * from './hooks/useMergeRejections';

// Full-Text Search hooks
export * from './hooks/useSearch';

// Hybrid Query hooks
export * from './hooks/useHybridQuery';

// Vector Search hooks
export * from './hooks/useVectorSearch';

// Hybrid Search hooks
export * from './hooks/useHybridSearch';

// Hybrid Search subscription hooks
export * from './hooks/useHybridSearchSubscribe';
