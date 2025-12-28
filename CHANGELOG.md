# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2025-12-28

### Added

#### New CRDTs and Data Structures
- **PN Counter** (Phase 5.2) - Positive-Negative counter CRDT supporting increment/decrement operations with eventual consistency
- **Event Journal** (Phase 5.04) - Append-only event log for event sourcing patterns with full CRDT synchronization
- **Entry Processor** (Phase 5.03) - Atomic server-side map operations with sandboxed execution via isolated-vm

#### API Enhancements
- **Custom Conflict Resolvers** (Phase 5.05) - Register custom conflict resolution strategies for domain-specific merge logic
- **Delta Updates for Subscriptions** (Phase 5.1) - Incremental updates instead of full data snapshots for improved performance
- **Offline Persistence for PNCounter** (Phase 5.02) - Full offline support for counter operations

#### React Hooks
- `usePNCounter` - Hook for working with PN Counter in React applications
- `useEventJournal` - Hook for subscribing to Event Journal streams
- Enhanced `useQuery` with delta update support

#### Documentation
- PNCounter guide and API reference
- Entry Processor guide and API reference
- Event Journal guide and API reference
- Conflict resolution blog post with ML/AI integration examples

### Fixed
- **react**: Added `maxChanges` option to prevent memory leaks in change tracking
- **server**: Mark isolated-vm as external for bundling
- **server**: Add production warning when isolated-vm is unavailable

### Changed
- **client**: Replace console.log with logger in QueryHandle for better debugging

## [0.3.0] - 2025-12-25

### Added
- Phase 4 completion: Cluster support with split-brain protection
- FailureDetector and FencingManager for cluster resilience
- Cluster Client documentation

### Fixed
- Test alignment with current implementation

## [0.2.1] - 2025-12-24

### Fixed
- Minor bug fixes and stability improvements

## [0.2.0] - 2025-12-23

### Added
- Phase 0 complete: Core CRDT implementation
- LWWMap and ORMap with HLC timestamps
- MerkleTree for efficient delta sync
- Basic client-server synchronization
