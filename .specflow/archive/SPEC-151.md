---
id: SPEC-151
type: bugfix
status: done
priority: P1
complexity: small
created: 2026-03-25
source: TODO-186
delta: true
---

# Fix deployment.mdx — Remove Non-Functional Env Vars and Config

## Context

`deployment.mdx` shows `TOPGUN_PORT`, `TOPGUN_TLS_*`, `TOPGUN_CLUSTER_PORT`, and `TOPGUN_CLUSTER_TLS_*` environment variables in Docker Compose and Kubernetes config examples. None of these are parsed by the Rust server. Users copying these configs get no TLS, wrong ports, and no cluster communication.

The actual env vars read by the Rust server (`packages/server-rust/src/`):

| Env Var | Source File | Purpose |
|---------|-------------|---------|
| `PORT` | `bin/test_server.rs` | Bind port (default: OS-assigned) |
| `DATABASE_URL` | `storage/datastores/postgres.rs` | PostgreSQL connection string |
| `JWT_SECRET` | `network/module.rs` | JWT authentication secret |
| `RUST_LOG` | `service/middleware/observability.rs` | Log filter level |
| `TOPGUN_LOG_FORMAT` | `service/middleware/observability.rs` | `json` or human-readable |
| `TOPGUN_ADMIN_USERNAME` | `network/handlers/admin.rs` | Admin panel username |
| `TOPGUN_ADMIN_PASSWORD` | `network/handlers/admin.rs` | Admin panel password |
| `TOPGUN_ADMIN_DIR` | `network/module.rs` | Admin SPA static files directory |

Priority is P1 because Docker/K8s configs silently fail — users believe they have TLS enabled when they do not.

## Delta

### MODIFIED
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — Fix non-functional env vars and add planned banners
  - Replace `TOPGUN_PORT` with `PORT` in TLS Docker Compose and K8s Deployment code blocks
  - Add "planned" banner above TLS Docker Compose section (already partially present at line 385-387; verify it is sufficient)
  - Add "planned" banner above Kubernetes TLS Deployment section (k8sDeploymentTlsCode)
  - Add "planned" banner above Kubernetes TLS Secret section (k8sSecretCode)
  - Remove the generic AlertBox at line 11 (replaced by specific per-section banners)
  - Remove the unused `AlertBox` import at line 9 (no longer referenced after deleting usage at line 11)
  - Update TLS code block titles to include "(planned)" suffix

## Requirements

### File: `apps/docs-astro/src/content/docs/guides/deployment.mdx`

**R1: Remove top-level generic AlertBox (line 11) and its import (line 9)**
Delete the `<AlertBox variant="warning" ... />` at line 11. Per-section banners replace it. Also remove the `AlertBox` import on line 9 — it will no longer be referenced anywhere in the file after the usage at line 11 is deleted.

**R2: Basic Docker Compose section — use only real env vars**
The `dockerComposeCode` variable (lines 16-54) is already mostly correct. Verify it uses `RUST_LOG` and `DATABASE_URL` (it does). No `TOPGUN_PORT` appears here currently — confirmed accurate. No changes needed to this block.

**R3: TLS Docker Compose section — planned banner**
A yellow banner already exists at lines 385-387 above the TLS Docker Compose block. Verify it clearly states:
- The `TOPGUN_TLS_*` and `TOPGUN_CLUSTER_*` env vars are not yet parsed by the server
- TLS is currently configured programmatically via `TlsConfig` when embedding the server
- A production CLI binary with env var config is planned

The existing banner text at line 386 covers this. Keep it as-is.

Additionally, in the `dockerComposeTlsCode` variable (lines 56-106):
- Replace `TOPGUN_PORT: 443` with `PORT: 443` on line 87
- Keep the rest of the non-functional TLS vars as illustrative of the planned config (the banner disclaims them)

**R4: Kubernetes TLS Secret section — add planned banner**
Add a yellow "planned" banner div above the `k8sSecretCode` block (before line 419), matching the style of the TLS Docker Compose banner. Text: "The Kubernetes TLS configuration shown below is planned for the future production binary. The env vars (`TOPGUN_TLS_*`, `TOPGUN_CLUSTER_*`) are not yet parsed by the server. For current TLS setup, configure `TlsConfig` programmatically when embedding the server. See the <a href="/docs/reference/server">Server API</a> for the current programmatic approach."

**R5: Kubernetes Deployment with TLS section — add planned banner**
Add a yellow "planned" banner div above the `k8sDeploymentTlsCode` block (before line 425), matching the same style. Text: same as R4.

In the `k8sDeploymentTlsCode` variable (lines 143-192):
- Replace `TOPGUN_PORT` with `PORT` on line 167-168

**R6: Ensure the title attribute on TLS code blocks says "(planned)"**
- `dockerComposeTlsCode` block title at line 391 already says `"docker-compose.tls.yml (planned)"` — keep
- `k8sDeploymentTlsCode` block title at line 427: change to `"k8s/deployment.yaml (planned)"`
- `k8sSecretCode` block title at line 421: change to `"k8s/tls-secret.yaml (planned)"`

## Acceptance Criteria

1. The generic `AlertBox` at the top of the file is removed
2. The `AlertBox` import is removed from the file (no dead imports remain)
3. The basic Docker Compose section (`dockerComposeCode`) contains only env vars that the Rust server actually parses: `RUST_LOG`, `DATABASE_URL`
4. `TOPGUN_PORT` does not appear anywhere in the file; replaced with `PORT` where a port env var is shown
5. A yellow "planned" banner appears above EACH of: TLS Docker Compose block, K8s TLS Secret block, K8s TLS Deployment block
6. Each planned banner states that the env vars are not yet parsed and links to programmatic `TlsConfig` approach
7. Code block titles for TLS/cluster sections include "(planned)" suffix
8. The basic Docker section and basic Docker Compose section remain unchanged (they are accurate)
9. The page renders without build errors (`pnpm --filter docs-astro build` or dev server check)

## Constraints

- Do NOT remove the TLS/cluster code examples entirely — they show the planned configuration direction
- Do NOT add new env vars to the basic Docker Compose section (it is already correct)
- Do NOT modify sections below "Serverless Deployment" — they are out of scope
- Keep the existing banner style (yellow background, matching dark mode classes)

## Assumptions

- The yellow banner div pattern used at lines 385-387 is the correct "planned" banner style for this page (reuse it for K8s sections)
- `PORT` is the correct replacement for `TOPGUN_PORT` based on `test_server.rs` parsing `std::env::var("PORT")`
- The docs site package filter name is `docs-astro` based on the workspace name

## Audit History

### Audit v1 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~8% total (1 MDX file, straightforward text edits)

**Delta validation:** 1/1 entries valid

**Strategic fit:** Aligned with project goals — fixes misleading documentation that causes silent deployment failures.

**Project compliance:** Honors PROJECT.md decisions. This is an MDX docs file, not Rust code — Language Profile does not apply.

**Recommendations:**
1. The Delta section sub-bullet "Replace `TOPGUN_PORT` with `PORT` in basic Docker Compose code block" is inaccurate — `TOPGUN_PORT` does not appear in `dockerComposeCode`. The Delta has been corrected in this audit to say "TLS Docker Compose and K8s Deployment code blocks" instead.
2. R4 and R5 banner text should include a link to `/docs/reference/server` (matching the existing TLS Docker Compose banner style) to satisfy AC5 which requires banners to "link to programmatic TlsConfig approach." R4 text has been updated in this audit to include the link.
3. Consider also removing the unused `AlertBox` import on line 9 after deleting the AlertBox usage at line 11, to avoid a dead import.
4. The `TOPGUN_CLUSTER_PORT` variable on line 88 of `dockerComposeTlsCode` and line 169 of `k8sDeploymentTlsCode` are also non-functional env vars. Since the banner disclaims them as planned, this is acceptable, but worth noting for awareness.

**Comment:** Well-structured spec with clear requirements, accurate line references, and good separation of what exists vs. what needs changing. The small fixes noted above (Delta wording, banner link, dead import) have been applied or flagged. Ready for implementation.

### Response v1 (2026-03-25)
**Applied:** All recommendations from Audit v1

**Changes:**
1. [✓] Delta wording correction — already reflected in the spec (Delta sub-bullet reads "TLS Docker Compose and K8s Deployment code blocks," not "basic Docker Compose code block"). No further change needed.
2. [✓] R4 and R5 banner link to `/docs/reference/server` — R4 already contains the link in its banner text; R5 says "Text: same as R4," which inherits the link. No further change needed.
3. [✓] Remove unused `AlertBox` import — added to R1 (now "R1: Remove top-level generic AlertBox (line 11) and its import (line 9)"), added to Delta sub-bullets, and added as AC2. Acceptance Criteria renumbered accordingly.
4. [✗] `TOPGUN_CLUSTER_PORT` awareness note — informational only; no spec change required. The existing banners disclaim all non-functional vars; this is acceptable per the audit itself.

### Audit v2 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~8% total (1 MDX file, straightforward text edits)

**Delta validation:** 1/1 entries valid

**Strategic fit:** Aligned with project goals -- fixes misleading documentation that causes silent deployment failures.

**Project compliance:** Honors PROJECT.md decisions. This is an MDX docs file, not Rust code -- Language Profile does not apply.

**Comment:** Post-revision re-audit. All v1 recommendations were addressed: Delta wording is accurate, R4/R5 banner text includes Server API link, AlertBox import removal is captured in R1/Delta/AC2. Line references verified against actual file content -- all 12 checked references match. The three `TOPGUN_PORT` occurrences (lines 11, 87, 167) are each handled by R1, R3, and R5 respectively, satisfying AC4. Spec is clear, complete, and ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-25
**Commits:** 1

### Files Created
None.

### Files Modified
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — Removed AlertBox import and usage, replaced TOPGUN_PORT with PORT in TLS code blocks, added yellow planned banners above K8s TLS Secret and K8s TLS Deployment sections, updated K8s TLS code block titles to include "(planned)" suffix

### Files Deleted
None.

### Acceptance Criteria Status
- [x] The generic `AlertBox` at the top of the file is removed
- [x] The `AlertBox` import is removed from the file (no dead imports remain)
- [x] The basic Docker Compose section (`dockerComposeCode`) contains only env vars that the Rust server actually parses: `RUST_LOG`, `DATABASE_URL`
- [x] `TOPGUN_PORT` does not appear anywhere in the file; replaced with `PORT` where a port env var is shown
- [x] A yellow "planned" banner appears above EACH of: TLS Docker Compose block, K8s TLS Secret block, K8s TLS Deployment block
- [x] Each planned banner states that the env vars are not yet parsed and links to programmatic `TlsConfig` approach
- [x] Code block titles for TLS/cluster sections include "(planned)" suffix
- [x] The basic Docker section and basic Docker Compose section remain unchanged (they are accurate)
- [x] The page renders without build errors (build passes: 65 pages built in 18.75s)

### Deviations
1. [Rule 1 - Bug] The asterisks in `TOPGUN_TLS_*` and `TOPGUN_CLUSTER_*` inside `<code>` tags in new K8s banner divs caused MDX to parse them as markdown emphasis, breaking the build. Fixed by using JSX string expressions `{'TOPGUN_TLS_*'}` instead of bare text inside `<code>` elements. The pre-existing Docker Compose TLS banner (which used the same pattern) was unaffected because it had only one asterisk-pattern, while the new banners had two in close proximity triggering the emphasis match.

### Notes
- The existing Docker Compose TLS banner (lines 382-384) already correctly used `<code>TOPGUN_TLS_*</code>` without JSX escaping and built fine previously — it continues to work as-is. Only the two new K8s banners needed the JSX expression workaround due to having two asterisk-containing patterns adjacent to each other.

---

## Review History

### Review v1 (2026-03-25 00:00)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: Generic AlertBox removed — no AlertBox element exists anywhere in the file
- [✓] AC2: AlertBox import removed — no AlertBox import present; only `CodeBlock` and lucide-react icons imported
- [✓] AC3: `dockerComposeCode` uses only `RUST_LOG` and `DATABASE_URL` — no spurious env vars
- [✓] AC4: `TOPGUN_PORT` does not appear anywhere in the file — grep confirms zero matches; replaced with `PORT` in both TLS Docker Compose (line 84) and K8s deployment (line 164) blocks
- [✓] AC5: Yellow "planned" banners present above all three TLS blocks — Docker Compose TLS (lines 382-384), K8s TLS Secret (lines 416-418), K8s TLS Deployment (lines 426-428)
- [✓] AC6: All banners state env vars are not yet parsed and link to `/docs/reference/server` — Docker Compose banner mentions `TOPGUN_TLS_*`; K8s banners mention both `TOPGUN_TLS_*` and `TOPGUN_CLUSTER_*` using JSX string expressions to avoid MDX asterisk parsing issues
- [✓] AC7: All TLS code block titles include "(planned)" — `docker-compose.tls.yml (planned)` (line 387), `k8s/tls-secret.yaml (planned)` (line 421), `k8s/deployment.yaml (planned)` (line 431)
- [✓] AC8: Basic Docker and Docker Compose sections unchanged — `dockerBuildCode` and `dockerComposeCode` unmodified
- [✓] AC9: Build passes — Execution Summary confirms 65 pages built successfully in 18.75s
- [✓] Deviation handled correctly — JSX string expressions `{'TOPGUN_TLS_*'}` and `{'TOPGUN_CLUSTER_*'}` resolve the MDX emphasis-parsing issue in the new K8s banners; pre-existing Docker Compose banner unaffected
- [✓] No sections below "Serverless Deployment" modified — constraint honored
- [✓] No dead imports remain in the file

**Summary:** All 9 acceptance criteria are met. The implementation correctly removes the misleading `TOPGUN_PORT` usage, adds contextual "planned" banners to each TLS section, keeps the TLS examples as illustrative of future config direction, and the build deviation (JSX escaping for asterisks) was identified and resolved cleanly.

---

## Completion

**Completed:** 2026-03-25
**Total Commits:** 1
**Review Cycles:** 1

### Outcome

Fixed misleading deployment documentation by removing non-functional `TOPGUN_PORT` env vars, replacing with `PORT`, and adding yellow "planned" banners above all TLS/cluster configuration sections that use env vars not yet parsed by the Rust server.

### Key Files

- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — Deployment guide with corrected env vars and planned feature banners

### Changes Applied

**Modified:**
- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — Removed AlertBox import and usage, replaced `TOPGUN_PORT` with `PORT` in TLS code blocks, added planned banners above K8s TLS Secret and K8s TLS Deployment sections, updated code block titles with "(planned)" suffix

### Deviations from Delta

- `apps/docs-astro/src/content/docs/guides/deployment.mdx` — New K8s banner divs required JSX string expressions `{'TOPGUN_TLS_*'}` instead of bare text inside `<code>` elements to avoid MDX asterisk emphasis-parsing; not anticipated in spec

### Patterns Established

None — followed existing patterns.

### Spec Deviations

None — implemented as specified (MDX escaping workaround was a build fix, not a spec deviation).
