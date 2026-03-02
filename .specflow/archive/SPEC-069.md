---
id: SPEC-069
type: chore
status: done
priority: P1
complexity: small
created: 2026-02-28
---

# Replace BSL 1.1 License with Apache 2.0

## Context

TopGun currently uses the Business Source License 1.1 (BSL 1.1). This blocks community adoption because BSL is perceived as non-open-source by many developers and organizations. Switching to Apache 2.0 removes this adoption barrier. Enterprise features planned for v3.0 will have their own BSL LICENSE in the `enterprise/` directory when that work begins (not part of this spec).

## Task

Replace all BSL 1.1 license references with Apache License 2.0 across the entire project. Replace the existing BSL 1.1 LICENSE file with the standard Apache 2.0 LICENSE file. Update the NOTICE file to reflect the new license.

## Requirements

### Files to Replace

1. **`LICENSE`** (project root)
   - Replace existing BSL 1.1 text (98 lines) with the complete Apache License 2.0 text
   - Verbatim from https://www.apache.org/licenses/LICENSE-2.0.txt

### Files to Modify

2. **`NOTICE`** (project root)
   - Change license reference from BSL 1.1 to Apache License 2.0
   - Remove the BSL-specific sections (WHAT IS PERMITTED, WHAT IS NOT PERMITTED, CHANGE LICENSE, COMMERCIAL LICENSING)
   - Keep copyright line: `Copyright (c) 2024 TopGun Contributors`
   - Keep third-party notices section
   - Follow standard Apache 2.0 NOTICE file format

3. **`Cargo.toml`** (project root, line 11)
   - Change `license = "BSL-1.1"` to `license = "Apache-2.0"`

4. **`package.json`** (project root, line 89)
   - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

5. **`packages/core/package.json`**
   - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

6. **`packages/client/package.json`**
   - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

7. **`packages/server/package.json`**
   - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

8. **`packages/react/package.json`**
   - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

9. **`packages/adapters/package.json`**
   - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

10. **`packages/adapter-better-auth/package.json`**
    - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

11. **`packages/mcp-server/package.json`**
    - Change `"license": "BSL-1.1"` to `"license": "Apache-2.0"`

12. **`CONTRIBUTING.md`** (project root, line 244)
    - Change "contributions will be licensed under the BSL-1.1 license" to reference Apache License 2.0

13. **`packages/mcp-server/README.md`** (line 199)
    - Change `BSL-1.1` license reference to `Apache-2.0`

### Files NOT Modified

- **`packages/native/package.json`** -- remains `"license": "MIT"` (native addon with separate license)
- Per-crate `Cargo.toml` files in `packages/core-rust/` and `packages/server-rust/` -- they inherit `license` from `[workspace.package]`, so no individual changes needed
- **`.specflow/reference/STRATEGIC_RECOMMENDATIONS.md`** -- references BSL 1.1 only in directory structure examples (lines 55, 74-75) illustrating a hypothetical `enterprise/` layout; these are not project license declarations and do not constitute BSL-1.1 adoption claims. Exclude from grep verification of AC #7.

## Acceptance Criteria

1. A `LICENSE` file exists at the project root containing the complete, unmodified Apache License 2.0 text
2. The `NOTICE` file references Apache License 2.0 (not BSL 1.1) and follows standard Apache NOTICE format
3. `Cargo.toml` workspace `license` field is `"Apache-2.0"`
4. Root `package.json` `license` field is `"Apache-2.0"`
5. All 7 per-package `package.json` files (core, client, server, react, adapters, adapter-better-auth, mcp-server) have `"license": "Apache-2.0"`
6. `packages/native/package.json` still has `"license": "MIT"` (unchanged)
7. No other files in the repository reference BSL-1.1 as the project license (grep verification — exclude `.specflow/reference/STRATEGIC_RECOMMENDATIONS.md` which contains only illustrative examples)

## Constraints

- Do NOT change `packages/native/package.json` license (it is correctly MIT)
- Do NOT create an `enterprise/` directory or BSL license for enterprise features (deferred to v3.0)
- Do NOT add license headers to source files (that is a separate task if desired)
- Use the exact SPDX identifier `Apache-2.0` in Cargo.toml and package.json files

## Assumptions

- Copyright year stays as 2024 (original project start year)
- No SPDX license headers are added to individual source files (only the root LICENSE file and metadata fields are updated)
- The NOTICE file follows the minimal Apache 2.0 NOTICE format (project name, copyright, license reference, third-party notice)

## Audit History

### Audit v1 (2026-02-28)
**Status:** NEEDS_REVISION

**Context Estimate:** ~10% total (11 small file edits + 1 file creation, all trivial single-line changes)

**Critical:**
1. **Missing files: `CONTRIBUTING.md` and `packages/mcp-server/README.md` reference BSL-1.1 but are not listed for modification.** `CONTRIBUTING.md` line 244 says "contributions will be licensed under the BSL-1.1 license" and `packages/mcp-server/README.md` line 199 says "BSL-1.1". These must be updated to satisfy AC #7 ("No other files in the repository reference BSL-1.1 as the project license"). Add both files to the "Files to Modify" section with instructions to change the license reference to Apache-2.0.
2. **LICENSE file already exists -- spec says "Files to Create" but should say "Files to Replace".** The root `LICENSE` file currently contains the full BSL 1.1 text (98 lines). The spec should clarify this is a replacement of the existing file, not creation of a new file where none existed. Move to "Files to Modify" or change the heading to "Files to Replace" to avoid executor confusion.

**Recommendations:**
3. [Compliance] The `.specflow/reference/STRATEGIC_RECOMMENDATIONS.md` document references BSL 1.1 in its directory structure examples (lines 55, 74-75). These are strategic reference docs not project license declarations, so they arguably do not violate AC #7. However, consider adding a note to the "Files NOT Modified" section acknowledging this to prevent implementor uncertainty during grep verification.

### Response v1 (2026-02-28)
**Applied:** All critical issues and recommendations (items 1, 2, 3)

**Changes:**
1. [✓] Missing files CONTRIBUTING.md and packages/mcp-server/README.md — Added both as items 12 and 13 in "Files to Modify" with specific instructions to update the BSL-1.1 license references to Apache License 2.0 / Apache-2.0.
2. [✓] LICENSE "Files to Create" heading — Renamed section to "Files to Replace" and updated the item description to clarify it replaces the existing BSL 1.1 text (98 lines) rather than creating a new file. Also updated the Task paragraph to match.
3. [✓] STRATEGIC_RECOMMENDATIONS.md compliance note — Added `.specflow/reference/STRATEGIC_RECOMMENDATIONS.md` to the "Files NOT Modified" section with an explanation that its BSL 1.1 references are illustrative directory structure examples, not project license declarations. AC #7 updated to reference the exclusion.

### Audit v2 (2026-02-28)
**Status:** APPROVED

**Context Estimate:** ~19% total (1 file replacement + 12 single-line edits, all trivial changes)

**Comment:** Spec is clear, complete, and implementable. All v1 critical issues have been resolved: CONTRIBUTING.md and mcp-server/README.md are now listed for modification, the LICENSE section is correctly labeled "Files to Replace", and STRATEGIC_RECOMMENDATIONS.md is documented in the exclusion list. Grep verification of the actual codebase confirms every BSL-1.1 reference in project files is accounted for. The 13 file changes are well-scoped for a "small" complexity spec. File count (13) exceeds the Language Profile limit (5), but the Language Profile explicitly applies only to `packages/core-rust/` and `packages/server-rust/` -- this spec modifies neither, so the limit does not apply. Strategic fit is strong: aligns with the project roadmap (TODO-094) and removes an adoption barrier. Project compliance verified -- no constraint violations.

---

## Execution Summary

**Executed:** 2026-02-28
**Commits:** 3

### Files Modified
- `LICENSE` — replaced BSL 1.1 text (98 lines) with complete Apache License 2.0 text
- `NOTICE` — replaced BSL 1.1 format with standard Apache 2.0 NOTICE format (copyright, license reference, third-party notices)
- `Cargo.toml` — changed `license = "BSL-1.1"` to `license = "Apache-2.0"`
- `package.json` — changed `"license": "BSL-1.1"` to `"license": "Apache-2.0"`
- `packages/core/package.json` — changed license to `Apache-2.0`
- `packages/client/package.json` — changed license to `Apache-2.0`
- `packages/server/package.json` — changed license to `Apache-2.0`
- `packages/react/package.json` — changed license to `Apache-2.0`
- `packages/adapters/package.json` — changed license to `Apache-2.0`
- `packages/adapter-better-auth/package.json` — changed license to `Apache-2.0`
- `packages/mcp-server/package.json` — changed license to `Apache-2.0`
- `CONTRIBUTING.md` — changed BSL-1.1 license reference to Apache License 2.0
- `packages/mcp-server/README.md` — changed BSL-1.1 to Apache-2.0

### Files Deleted
(none)

### Acceptance Criteria Status
- [x] AC1: LICENSE file exists at project root with complete Apache License 2.0 text
- [x] AC2: NOTICE file references Apache License 2.0 and follows standard Apache NOTICE format
- [x] AC3: Cargo.toml workspace license field is "Apache-2.0"
- [x] AC4: Root package.json license field is "Apache-2.0"
- [x] AC5: All 7 per-package package.json files have "license": "Apache-2.0"
- [x] AC6: packages/native/package.json still has "license": "MIT" (unchanged)
- [x] AC7: No other project files reference BSL-1.1 as project license (grep verified; .specflow/ files and STRATEGIC_RECOMMENDATIONS.md excluded per spec)

### Deviations
(none)

### Notes
- All 13 file changes applied cleanly with no issues
- Grep verification confirmed all remaining BSL-1.1 references are in .specflow/ management files (specs, state, todos, archive) and the excluded STRATEGIC_RECOMMENDATIONS.md illustrative examples — none are project license declarations

---

## Review History

### Review v1 (2026-02-28)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: LICENSE file contains complete Apache License 2.0 text (202 lines, verbatim standard text including all 9 sections and Appendix)
- [✓] AC2: NOTICE file references Apache License 2.0, follows standard format — project name, copyright 2024, license URL, third-party notices section; no BSL-specific sections remain
- [✓] AC3: Cargo.toml workspace `license = "Apache-2.0"` at line 11
- [✓] AC4: Root package.json `"license": "Apache-2.0"` at line 89
- [✓] AC5: All 7 per-package package.json files updated — core (line 42), client (line 38), server (line 53), react (line 50), adapters (line 41), adapter-better-auth (line 40), mcp-server (line 45)
- [✓] AC6: packages/native/package.json retains `"license": "MIT"` (unchanged)
- [✓] AC7: Broad grep across repository confirms zero BSL-1.1 references in project source or configuration files; all remaining matches are in .specflow/ management files (specs, state, todos, archive) and the excluded STRATEGIC_RECOMMENDATIONS.md illustrative examples — none are project license declarations
- [✓] CONTRIBUTING.md line 244 now reads "Apache License 2.0" — no BSL-1.1 reference
- [✓] packages/mcp-server/README.md line 199 now reads "Apache-2.0" — no BSL-1.1 reference
- [✓] No enterprise/ directory created (constraint respected)
- [✓] No license headers added to source files (constraint respected)
- [✓] SPDX identifier Apache-2.0 used correctly in all metadata fields

**Summary:** All 13 file changes are correctly implemented. Every acceptance criterion is satisfied. The implementation is clean, complete, and leaves no stray BSL-1.1 references in project files.

---

## Completion

**Completed:** 2026-02-28
**Total Commits:** 3
**Audit Cycles:** 2
**Review Cycles:** 1
