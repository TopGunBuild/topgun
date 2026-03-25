---
id: SPEC-154
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-03-25
source: TODO-189
delta: true
---

# Fix cluster-replication.mdx — Remove False Env Vars, Consistency Modes, and Metrics

## Context

`cluster-replication.mdx` contains multiple categories of false information:

1. **Non-existent env vars:** `TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`, `TOPGUN_NODE_ID`, `TOPGUN_CONSISTENCY`, `TOPGUN_REPLICATION`, `TOPGUN_PEERS` — none are parsed by the Rust server. Users copying the Cluster Setup or Docker Compose examples get no cluster communication.

2. **Non-existent consistency modes:** Documents QUORUM and STRONG consistency — only eventual consistency exists. The comparison table and best practices reference these modes as if selectable.

3. **Non-existent metrics:** `topgun_replication_queue_size`, `topgun_replication_pending_acks`, `topgun_replication_lag_ms`, `topgun_replication_healthy`, `topgun_replication_unhealthy_nodes` — the Rust server has no Prometheus metrics at all. The distributed subscription metrics (`topgun_distributed_sub_*`) are also non-existent.

4. **Non-existent components:** `ReadReplicaHandler`, `ReplicationPipeline.getHealth()`, `RepairScheduler`, `MerkleTreeManager` — none exist in the codebase.

5. **Incorrect gossip terminology:** Docs say HELLO/MEMBER_LIST messages; actual cluster uses `JoinRequest`/`JoinResponse` via `ClusterMessage` enum.

**What IS accurate** (verified in `packages/server-rust/src/cluster/`):
- 271 partitions with consistent hashing (`assignment.rs`)
- `backup_count` configuration (`types.rs`, default: 1)
- Phi Accrual failure detector with `phi_threshold: 8.0`, `heartbeat_interval_ms: 1000` (`failure_detector.rs`)
- Partition rebalancing on node failure (`resilience.rs`, `migration.rs`)
- Gossip-based join discovery via `JoinRequest`/`JoinResponse` (`messages.rs`)

Priority is P1 because cluster setup instructions silently fail — users believe they have a working cluster when env vars are ignored.

## Delta

### MODIFIED
- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` — Fix false cluster documentation
  - Cluster Setup: Add yellow "planned" banner above env-var-based cluster config; note that cluster is currently configured programmatically via `ClusterConfig` struct
  - Gossip Discovery: Fix message names from HELLO/MEMBER_LIST to JoinRequest/JoinResponse; add planned banner above env var usage
  - Consistency Levels: Add yellow "planned" banner stating only EVENTUAL exists; mark QUORUM/STRONG rows as "(planned)" in comparison table
  - Docker Compose: Add yellow "planned" banner stating these env vars are not parsed; entire example is aspirational
  - Read Replicas: Add yellow "planned" banner (ReadReplicaHandler does not exist)
  - Monitoring/Prometheus Metrics: Remove the `metricsCode` block entirely and the replication metrics section (none exist)
  - Health Checks: Remove `ReplicationHealth` TypeScript interface (ReplicationPipeline does not exist)
  - Distributed Subscriptions Metrics: Remove the `topgun_distributed_sub_*` metrics code block (none exist); add note that metrics are planned
  - Anti-Entropy Repair: Fix references to non-existent RepairScheduler/MerkleTreeManager — describe as architectural design (Merkle tree comparison is real in core, but RepairScheduler is not implemented)
  - Best Practices: Remove references to QUORUM/STRONG consistency; rewrite tips to reflect eventual consistency reality

## Requirements

### File: `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx`

**R1: Add planned banner above Cluster Setup section**
Add a yellow "planned" banner div immediately before the `clusterSetupCode` CodeBlock (before line 210). Text: "Environment variable-based cluster configuration (`TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`) is planned for the production CLI binary. The cluster is currently configured programmatically via the `ClusterConfig` struct when embedding the server. See the <a>Server API</a> for the current approach."

Also add a comment line inside `clusterSetupCode` itself (similar to R3's approach for `consistencyLevelsCode`): `# These env vars are planned — the server does not currently parse them.`

**R2: Fix gossip discovery message names**
In `gossipProtocolCode` (lines 70-85):
- Replace "Sends HELLO message with its info" with "Sends JoinRequest with its node info"
- Replace "Receives MEMBER_LIST with all known members" with "Receives JoinResponse with cluster membership"
- Add planned banner above the gossip code block noting that the env vars (`TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`) shown are planned

**R3: Mark QUORUM/STRONG as planned in Consistency Levels**
- Add a yellow "planned" banner below the "TopGun supports three consistency levels" text stating: "Only EVENTUAL consistency is currently implemented. QUORUM and STRONG modes are planned for a future release."
- In the comparison table, append "(planned)" to QUORUM and STRONG rows
- In `consistencyLevelsCode` (lines 30-39), add a comment line: "# Currently only EVENTUAL is implemented. QUORUM and STRONG are planned."

**R4: Add planned banner above Docker Compose Example**
Add a yellow "planned" banner before the Docker Compose section (before line 335). Text: "The Docker Compose cluster example below uses environment variables (`TOPGUN_NODE_ID`, `TOPGUN_CONSISTENCY`, `TOPGUN_REPLICATION`, `TOPGUN_PEERS`) that are not yet parsed by the server. This example shows the planned configuration approach. Multi-node cluster setup currently requires programmatic configuration."

**R5: Add planned banner above Read Replicas section**
Add a yellow "planned" banner before the Read Replicas CodeBlock (before line 329). Text: "Read replica routing is planned. The server does not currently implement ReadReplicaHandler or configurable read preferences."

**R6: Remove non-existent replication metrics**
- Delete the entire `metricsCode` export variable (lines 164-178)
- Remove the "Prometheus Metrics" subsection under Monitoring (lines 347-353) that renders `metricsCode`
- Replace with a brief note: "Replication-specific Prometheus metrics are planned. The server does not currently expose replication metrics."

**R7: Remove non-existent Health Checks**
Delete the "Health Checks" subsection (lines 355-366) containing the `ReplicationHealth` TypeScript interface. `ReplicationPipeline` does not exist.

**R8: Remove non-existent distributed subscription metrics**
Delete the metrics code block in the Distributed Subscriptions section (lines 442-457) containing `topgun_distributed_sub_*` metrics. Replace with a note: "Distributed subscription metrics are planned for a future release."

**R9: Fix Anti-Entropy Repair references**
In `antiEntropyCode` (lines 56-68):
- Remove references to "RepairScheduler" and "MerkleTreeManager" as named components (they do not exist as implemented types)
- Rewrite to describe the architectural design: partition owners and backups compare Merkle tree roots to identify divergent data, then sync only differing keys

Also fix the "How It Works" HTML section (lines 272-311):
- Step 2 (line 289) references "RepairScheduler" — rewrite to remove this non-existent component name (e.g., "The system periodically compares Merkle roots between owner and backup nodes")

**R10: Fix Best Practices for eventual-only reality**
- Best Practice 1 ("Use Odd Node Counts"): Remove the QUORUM justification. Rewrite to: "Use multiple nodes for redundancy. The `backup_count` setting (default: 1) controls how many backup copies exist per partition."
- Best Practice 2 ("Match Consistency to Use Case"): Remove entirely (only one consistency mode exists)
- Best Practice 3 ("Monitor Replication Lag"): Remove the reference to `topgun_replication_lag_ms` metric (does not exist). Rewrite to general advice about monitoring cluster health.
- Best Practice 4 ("Enable Cluster TLS"): Add "(planned)" — TLS for inter-node communication is not yet configurable

**R11: Update top-of-page AlertBox**
The existing AlertBox (line 11) says "Some features on this page (consistency modes, configurable replication factor) are not yet available." Update to be more comprehensive: "Several features on this page are marked as planned, including: environment variable cluster configuration, QUORUM/STRONG consistency modes, read replica routing, and replication metrics. Sections describing real behavior (partitioning, gossip discovery, failure detection, anti-entropy design) are accurate."

**R12: Fix Data Flow acknowledgment step**
In the Data Flow section (line 255), the acknowledgment step currently reads "Acknowledgment: Based on consistency level" — this implies configurable consistency modes that do not exist. Rewrite to: "Acknowledgment: Eventual — the CRDT merge completes locally and the operation is acknowledged; no quorum coordination occurs."

## Acceptance Criteria

1. No env var that the Rust server does not parse (`TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`, `TOPGUN_NODE_ID`, `TOPGUN_CONSISTENCY`, `TOPGUN_REPLICATION`, `TOPGUN_PEERS`) appears without a yellow "planned" banner directly above its containing section
2. QUORUM and STRONG consistency modes are marked "(planned)" in the comparison table and a banner states only EVENTUAL is implemented
3. The `metricsCode` variable and its rendered Prometheus Metrics subsection are removed
4. The `ReplicationHealth` TypeScript interface / Health Checks subsection is removed
5. The `topgun_distributed_sub_*` metrics code block is removed
6. Gossip discovery comments use JoinRequest/JoinResponse (not HELLO/MEMBER_LIST)
7. ReadReplicaHandler section has a "planned" banner
8. Best practices do not reference QUORUM, STRONG, or non-existent metrics as if available
9. Anti-entropy repair section does not reference RepairScheduler or MerkleTreeManager as named implemented components
10. The top-of-page AlertBox is updated to summarize which features are planned vs. real
11. The page renders without build errors (`pnpm --filter docs-astro build`)
12. Sections describing real behavior (partitioning with 271 partitions, backup_count, Phi Accrual failure detection, partition rebalancing) remain intact and unmodified in substance
13. The Data Flow acknowledgment step does not reference configurable consistency levels as if implemented; it reflects eventual consistency reality

## Constraints

- Do NOT remove the Docker Compose example, Cluster Setup example, or Consistency Levels section entirely — keep them with "planned" banners as they show the intended configuration direction
- Do NOT remove the Distributed Subscriptions architecture section (Subscription Protocol, Architecture diagram, Key Design Decisions, Node Disconnect Handling) — only remove the non-existent metrics block within it
- Do NOT modify the navigation links at the bottom of the page
- Use the same yellow banner div style as SPEC-151 established in deployment.mdx
- Use JSX string expressions `{'TOPGUN_*'}` for asterisk-containing env var patterns inside `<code>` tags to avoid MDX emphasis-parsing issues (lesson from SPEC-151)

## Assumptions

- The yellow "planned" banner pattern from deployment.mdx (yellow-50/yellow-900 background, yellow-500 border-l-4) is the correct style to reuse
- The Distributed Subscriptions architecture section (protocol, diagram, design decisions, disconnect handling) describes real or accurately-designed behavior and should be preserved — only the metrics are aspirational
- Anti-entropy repair via Merkle tree comparison is a real architectural design even though RepairScheduler/MerkleTreeManager are not implemented as named types — the concept is accurate, the specific component names are not
- The `AlertBox` import on line 9 is still needed (the component is used on line 11, just with updated text)

## Audit History

### Audit v1 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~12% total

**Delta validation:** 1/1 entries valid

**Strategic fit:** Aligned with project goals — fixes user-facing documentation that causes silent cluster setup failures

**Project compliance:** Honors PROJECT.md decisions (docs-only change, no Rust/TS code impact)

**Recommendations:**
1. R9 was missing coverage of the "How It Works" HTML section (line 289) which also references "RepairScheduler". This has been fixed inline in R9 during audit — the added paragraph now covers both the `antiEntropyCode` export variable AND the HTML step 2 text.
2. The Data Flow section (line 255) says "Acknowledgment: Based on consistency level" which implies configurable consistency. Consider rewording to "Acknowledgment: Eventual (CRDT merge completes locally)" or adding a planned note, since only eventual consistency exists.
3. The `clusterSetupCode` export (lines 13-28) still contains `TOPGUN_CLUSTER_PORT` and `TOPGUN_CLUSTER_SEEDS` env vars in the code block itself. While the planned banner above the rendered section addresses this visually, consider also adding a comment inside the code block (like R3 does for `consistencyLevelsCode`) noting these env vars are planned.

**Comment:** Well-structured spec with thorough identification of false documentation. All 11 requirements map cleanly to the 5 categories of problems identified in Context. Line number references verified accurate against current file. One gap in R9 was fixed during audit. Two minor recommendations remain.

### Response v1 (2026-03-25)
**Applied:** Audit recommendations 1, 2, and 3

**Changes:**
1. [✓] R9 coverage of "How It Works" HTML section — already fixed inline during audit; R9 requirement text was updated in-place to cover both `antiEntropyCode` and line 289 step 2 text. No further change needed.
2. [✓] Data Flow acknowledgment step implies configurable consistency — added R12 requiring the acknowledgment text to be rewritten to "Eventual — the CRDT merge completes locally and the operation is acknowledged; no quorum coordination occurs." Added acceptance criterion 13 to cover this.
3. [✓] `clusterSetupCode` contains unguarded env var references — amended R1 to require a comment line inside `clusterSetupCode` itself noting the env vars are planned (matching R3's pattern for `consistencyLevelsCode`).

### Audit v2 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~12% total

**Delta validation:** 1/1 entries valid

**Strategic fit:** Aligned with project goals

**Project compliance:** Honors PROJECT.md decisions

**Recommendations:**
1. [Minor] The Assumptions section states the banner style uses "yellow-500 border-l-4" but the actual deployment.mdx banners use `border border-yellow-100 dark:border-yellow-800 rounded-xl` (full border with rounded corners, not left-only border). The Constraints section correctly says "Use the same yellow banner div style as SPEC-151 established in deployment.mdx" which takes precedence, so this is cosmetic. The implementer should follow the Constraints directive and copy the actual deployment.mdx pattern.

**Comment:** All three recommendations from Audit v1 have been properly incorporated. R12 added for the Data Flow acknowledgment step, AC13 added to cover it, and R1 amended to include an inline comment in `clusterSetupCode`. Line references verified against the current 521-line file. Spec is complete, clear, and ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-25
**Commits:** 2

### Files Modified
- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` — Fixed all false documentation per R1-R12

### Files Created
(none)

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] 1. No unguarded env vars — all have yellow "planned" banners above their sections
- [x] 2. QUORUM and STRONG marked "(planned)" in table; banner states only EVENTUAL implemented
- [x] 3. `metricsCode` variable and Prometheus Metrics subsection removed
- [x] 4. `ReplicationHealth` TypeScript interface / Health Checks subsection removed
- [x] 5. `topgun_distributed_sub_*` metrics code block removed
- [x] 6. Gossip discovery uses JoinRequest/JoinResponse (not HELLO/MEMBER_LIST)
- [x] 7. ReadReplicaHandler section has "planned" banner
- [x] 8. Best practices do not reference QUORUM, STRONG, or non-existent metrics
- [x] 9. Anti-entropy section does not reference RepairScheduler or MerkleTreeManager as named components
- [x] 10. AlertBox updated to summarize planned vs. real features
- [x] 11. Page builds without errors (`pnpm --filter apps-docs-astro build` passes)
- [x] 12. Real behavior sections (271 partitions, backup_count, Phi Accrual, rebalancing) intact
- [x] 13. Data Flow acknowledgment step reflects eventual consistency, not configurable consistency

### Deviations
None — all requirements implemented as specified.

### Notes
- Banner style copied from deployment.mdx: `bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-100 dark:border-yellow-800 rounded-xl p-4 mb-4` (full border + rounded-xl, not border-l-4)
- The existing `border-l-4 border-yellow-500` "Important" box in the Data Flow section (lines 259-264) was intentionally left untouched per spec constraints — it is not part of any requirement
- `TOPGUN_*` env var patterns in banner text used plain `<code>` tags since they name specific variables (no asterisk wildcard), consistent with deployment.mdx approach

---

## Review History

### Review v1 (2026-03-25 21:08)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: No unguarded env vars — all sections containing `TOPGUN_CLUSTER_PORT`, `TOPGUN_CLUSTER_SEEDS`, `TOPGUN_NODE_ID`, `TOPGUN_CONSISTENCY`, `TOPGUN_REPLICATION`, `TOPGUN_PEERS` have yellow "planned" banners immediately above them (lines 200-202, 217-219, 341-343)
- [✓] AC2: QUORUM and STRONG marked "(planned)" in comparison table (lines 242-243); banner at lines 229-231 states only EVENTUAL is implemented; `consistencyLevelsCode` includes inline comment
- [✓] AC3: `metricsCode` variable completely absent from file; Prometheus Metrics subsection replaced with one-line planned note (line 361)
- [✓] AC4: `ReplicationHealth` and Health Checks subsection entirely absent — grep confirms zero occurrences
- [✓] AC5: `topgun_distributed_sub_*` metrics code block absent — grep confirms zero occurrences; replaced with planned note (line 435)
- [✓] AC6: Gossip discovery uses JoinRequest/JoinResponse (lines 85-86 of `gossipProtocolCode`); HELLO/MEMBER_LIST absent from file
- [✓] AC7: Read Replicas section has "planned" banner at lines 331-333
- [✓] AC8: Best Practices contains no QUORUM, STRONG, or non-existent metric references — three practices cover redundancy, cluster health monitoring, and TLS (planned)
- [✓] AC9: RepairScheduler and MerkleTreeManager absent from file; `antiEntropyCode` describes architectural design without named components; "How It Works" Step 2 (line 291) uses "The system periodically compares Merkle roots between owner and backup nodes"
- [✓] AC10: AlertBox (line 11) updated to comprehensive summary covering all planned features and what is real
- [✓] AC11: Build passes cleanly — `pnpm --filter apps-docs-astro build` completes with 65 pages built, exit 0
- [✓] AC12: 271 partitions (lines 44, 184), `backup_count` default 1 (line 451), `phiThreshold: 8` and `heartbeatInterval: 1000ms` (lines 96, 99), partition rebalancing (line 105, 460) all intact
- [✓] AC13: Data Flow step 5 reads "Acknowledgment: Eventual — the CRDT merge completes locally and the operation is acknowledged; no quorum coordination occurs." (line 257)
- [✓] R1 inline comment: `clusterSetupCode` starts with "# These env vars are planned — the server does not currently parse them." (line 13)
- [✓] Banner style: Uses `bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-100 dark:border-yellow-800 rounded-xl p-4 mb-4` matching deployment.mdx pattern (not border-l-4 from Assumptions)
- [✓] Constraints respected: Docker Compose, Cluster Setup, Consistency Levels sections kept with banners; Distributed Subscriptions architecture preserved; navigation links unmodified

**Minor:**
1. `readReplicaCode` (line 113) says "ReadReplicaHandler is active automatically" while the banner at lines 331-333 says "The server does not currently implement ReadReplicaHandler." The contradiction is visible to readers who read both the banner and the code block. R5 only required adding the banner, not editing the code block text, so this is within spec — but it creates a confusing user experience. The code block comment could read something like "# (planned) When implemented, ReadReplicaHandler will be active automatically."

**Summary:** All 13 acceptance criteria verified. The implementation correctly applies yellow planned banners to all sections with unimplemented env vars, removes all non-existent metrics and health check interfaces, fixes gossip terminology to JoinRequest/JoinResponse, rewrites best practices and data flow for eventual-consistency reality, and preserves all real-behavior content. The build passes cleanly. One minor cosmetic contradiction exists in `readReplicaCode` between the banner text and the code block comment, which is outside spec scope but worth noting.

### Fix Response v1 (2026-03-25)
**Applied:** Minor issue 1

**Fixes:**
1. [✓] `readReplicaCode` contradiction — Changed "ReadReplicaHandler is active automatically" to "Read replica routing is not yet implemented"; aligned code block text with planned banner
   - Commit: 2d33d22

### Review v2 (2026-03-25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Fix Verified:**
- [✓] Minor issue 1 resolved — `readReplicaCode` line 113 now reads `# Read replica routing is not yet implemented.` which is consistent with the planned banner at lines 331-333 ("The server does not currently implement ReadReplicaHandler or configurable read preferences."). The contradiction identified in Review v1 is fully resolved.

**Passed:**
- [✓] All 13 acceptance criteria from Review v1 remain satisfied — no regressions introduced by the fix
- [✓] The fix is minimal and targeted: only the one contradictory comment line was changed, no other content affected

**Summary:** The single minor fix from Review v1 was correctly applied. `readReplicaCode` no longer contradicts the planned banner above the Read Replicas section. No new issues introduced.

---

## Completion

**Completed:** 2026-03-25
**Total Commits:** 3
**Review Cycles:** 2

### Outcome

Fixed cluster-replication.mdx documentation by adding yellow "planned" banners above all sections referencing unimplemented features (env vars, QUORUM/STRONG consistency, read replicas, metrics), removing non-existent metrics/health checks, fixing gossip terminology, and rewriting best practices for eventual-consistency reality.

### Key Files

- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` — Cluster replication documentation with corrected planned/real distinction

### Changes Applied

**Modified:**
- `apps/docs-astro/src/content/docs/guides/cluster-replication.mdx` — Added planned banners (R1-R5), removed false metrics/health checks (R6-R8), fixed gossip names and anti-entropy references (R2, R9), updated AlertBox/Best Practices/Data Flow (R10-R12), fixed readReplicaCode contradiction

### Deviations from Delta

None — all modifications matched the Delta specification.

### Patterns Established

None — followed existing yellow "planned" banner pattern from deployment.mdx (SPEC-151).

### Spec Deviations

None — implemented as specified.
