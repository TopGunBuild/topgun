# SPEC-060e: Cluster Protocol — Resilience (Split-Brain, Graceful Leave, Mastership Claim)

```yaml
id: SPEC-060e
type: feature
status: deferred
priority: P1
complexity: medium
parent: SPEC-060
depends_on: [SPEC-060d, TODO-064]
created: 2026-02-22
todo: TODO-066
```

## Context

This is the fifth and final sub-specification of SPEC-060, covering the Wave 3 (Resilience) scope. It implements split-brain detection and CRDT auto-recovery, graceful leave protocol, mastership claim after master crash, and the heartbeat complaint protocol.

**Deferred:** This spec depends on SPEC-060d (migration service, for graceful leave partition draining) and TODO-064 (networking layer, for probing remote clusters and sending merge requests).

### Scope Preview

- Split-brain detection: master-centric seed probing per research Section 6.1
- CRDT auto-recovery on cluster merge per research Section 6.2
- Graceful leave protocol: LeaveRequest -> migrate partitions away -> remove from MembersView
- Mastership claim after master crash: oldest-member convention, majority agreement
- Heartbeat complaint protocol: non-master nodes report suspected failures to master for confirmation

### Acceptance Criteria (from parent SPEC-060)

30. Split-brain detected when master probes seed addresses and finds a different `MembersView` for the same `cluster_id`.
31. Larger cluster wins merge decision; tie broken by oldest master.
32. Graceful leave: node sends `LeaveRequest`, master migrates partitions away before removing.
33. Mastership claim: when master dies, all nodes independently compute the new master from `MembersView` and the new master takes over coordination.
34. Heartbeat complaint: non-master nodes report suspected failures to master for confirmation.

## Task

TBD -- Full requirements will be defined after SPEC-060d (migration service) and TODO-064 (networking layer) are complete.

## Constraints

1. Depends on SPEC-060d (migration service for partition draining during graceful leave).
2. Depends on TODO-064 (networking layer for seed probing and merge requests).
3. Must use the `ClusterMessage` variants (SplitBrainProbe, SplitBrainProbeResponse, MergeRequest, LeaveRequest, HeartbeatComplaint, ExplicitSuspicion) defined in SPEC-060a.

## Assumptions

1. CRDT merge during split-brain recovery reuses the same merge logic as migration (SPEC-060d).
2. The `SplitBrainMergeDecision` enum from the research document will be defined in this spec when detailed requirements are written.
