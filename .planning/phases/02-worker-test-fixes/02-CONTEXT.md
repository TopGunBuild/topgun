# Phase 2: Worker Test Fixes - Context

**Gathered:** 2026-01-18
**Status:** Ready for planning

<domain>
## Phase Boundary

Fix all skipped worker tests to restore CI confidence. Tests currently skipped:
- CRDTMergeWorker: 2 tests (large batch handling)
- MerkleWorker: 3 tests (large batches, rebuilds, 10k+ entries)
- SerializationWorker: 1 describe block (worker thread operations)
- DistributedSearch E2E: 1 describe block (entire test suite)

Phase delivers: All tests unskipped and passing in CI. No `test.skip` or `describe.skip` remains.

</domain>

<decisions>
## Implementation Decisions

### Coverage scope
- Unskip ALL tests — no skipped tests allowed in final state
- CI must be fully green with complete test coverage
- Tests must pass when run in full suite, not just individually
- Fix any isolation issues that cause suite-only failures

### Large batch tests
- Move large batch / stress tests to separate slow test file
- Allows main suite to run fast, slow suite can run separately in CI
- Affected tests: 10,000+ entry tests, large rebuilds, large batch handling

### E2E test handling
- DistributedSearch E2E gets its own plan (02-03)
- More complex than worker unit tests — warrants separate attention
- Worker unit tests grouped in plans 02-01 and 02-02

### Claude's Discretion
- Specific timeout values for slow tests
- Whether to fix test setup vs production code
- How to structure the slow test file
- Retry/flaky handling approach if needed

</decisions>

<specifics>
## Specific Ideas

No specific requirements — open to standard approaches for fixing worker thread tests.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 02-worker-test-fixes*
*Context gathered: 2026-01-18*
