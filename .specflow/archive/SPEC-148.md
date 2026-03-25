---
id: SPEC-148
type: docs
status: done
priority: P2
complexity: small
created: 2026-03-25
source: TODO-162
delta: true
---

# Add DataFusion SQL Query Documentation Page

## Context

SPEC-135a/b/c implemented the DataFusion SQL query engine behind the `datafusion` Cargo feature flag: `QueryBackend`/`SqlQueryBackend` traits, `SqlQuery`/`SqlQueryResp` wire messages, Arrow cache, MsgPack-to-Arrow conversion, `TopGunTableProvider`, `DataFusionBackend`, and `QueryService` integration. The feature is complete and functional but has no user-facing documentation. Developers need a guide explaining how to enable the feature, write SQL queries, understand supported syntax and data types, and be aware of current limitations.

## Delta

### ADDED
- `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` -- New documentation page covering DataFusion SQL query feature

### MODIFIED
- `apps/docs-astro/src/components/docs/DocsSidebar.tsx` -- Add "SQL Queries" entry to the Guides section

## Requirements

### R1: SQL Queries Guide Page

**File:** `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` (new)

Create an MDX documentation page following the existing guide page conventions (frontmatter with `title`, `description`, `order`; import of `CodeBlock` component and lucide icons; use of `FeatureCard` components where appropriate).

The page must contain these sections in order:

**1. Introduction**
- One-paragraph explanation: TopGun supports SQL queries via Apache DataFusion, enabling SELECT/WHERE/GROUP BY/ORDER BY/LIMIT on map data. Requires the `datafusion` Cargo feature flag.
- Note that this is a server-side feature -- SQL queries execute on the server, not on the client.

**2. Enabling DataFusion**
- Show how to build the server with the feature flag: `cargo build -p topgun-server --features datafusion`
- Explain that without the flag, only predicate-based queries (live queries) are available.
- Note that maps must have a registered schema (via `SchemaProvider`) before they can be queried via SQL.

**3. Wire Protocol**
- Explain the `SQL_QUERY` / `SQL_QUERY_RESP` message pair.
- Show the `SqlQueryPayload` structure: `sql` (query string) and `queryId` (correlation ID).
- Show the `SqlQueryRespPayload` structure: `queryId`, `columns` (column names), `rows` (array of row arrays as MsgPack values), `error` (optional error string).
- Provide a conceptual code example showing a client sending `SQL_QUERY` and receiving `SQL_QUERY_RESP`.

**4. Supported SQL Syntax**
- `SELECT` with column selection and `*`
- `WHERE` with comparison operators (`=`, `!=`, `<`, `>`, `<=`, `>=`), `AND`, `OR`, `NOT`, `IS NULL`, `IS NOT NULL`, `LIKE`, `IN`
- `GROUP BY` with aggregate functions: `COUNT(*)`, `SUM()`, `AVG()`, `MIN()`, `MAX()`
- `ORDER BY` with `ASC`/`DESC`
- `LIMIT` and `OFFSET`
- `DISTINCT`
- Aliases (`AS`)
- Arithmetic expressions in SELECT
- Provide 4-6 example queries covering common use cases (filtering, aggregation, sorting, counting)

**5. Supported Data Types**
- Table mapping MsgPack value types to Arrow/SQL types:
  - `Integer` -> `BIGINT` (Int64)
  - `Float` (f64) -> `DOUBLE` (Float64)
  - `String` -> `VARCHAR` (Utf8)
  - `Boolean` -> `BOOLEAN`
  - `Binary` -> `BYTEA` (Binary)
  - `Integer` (millis timestamp) -> `TIMESTAMP` (TimestampMillisecond)
  - `Array` -> `LIST`
  - `Nil` -> `NULL`
  - Complex/Map values -> JSON string (VARCHAR fallback)
- Note that every table includes a `_key` column (VARCHAR) representing the map entry key.

**6. Example Queries**
- Provide 3 complete, realistic examples with context (e.g., "Given a `products` map with fields `name`, `price`, `category`, `inStock`..."):
  - Basic filtering and sorting
  - Aggregation with GROUP BY
  - Combined filtering, aggregation, and ordering

**7. Limitations**
- Single-node only: SQL queries execute against the local node's data. Distributed SQL across cluster nodes is not yet supported.
- No cross-map JOINs: each SQL query targets a single map (table). JOINs between maps are planned for a future release.
- No live SQL subscriptions: SQL queries are one-shot request/response. For real-time updates, use predicate-based live queries.
- Schema required: maps without a registered schema cannot be queried via SQL (returns `SchemaRequired` error).
- No filter pushdown: DataFusion applies filters post-scan. Large maps may have higher query latency than predicate-based queries for simple filters.
- Arrow cache: results are cached per partition and invalidated on mutation. First query after mutations incurs a cache rebuild.

**Frontmatter:**
```yaml
---
title: SQL Queries
description: Query your data with SQL using Apache DataFusion. Supports SELECT, WHERE, GROUP BY, ORDER BY, and aggregate functions on map data.
order: 13
---
```

Note: `order: 13` is shared by several guides (schema, indexing, pub-sub). The frontmatter `order` field provides approximate content-collection ordering; the actual sidebar position is determined by SubItem placement in `DocsSidebar.tsx` (see R2).

### R2: Sidebar Entry

**File:** `apps/docs-astro/src/components/docs/DocsSidebar.tsx` (modify)

Add a `SubItem` entry for the SQL Queries page in the Guides section, positioned after "Live Queries" and before "Schema & Type Safety":

```tsx
<SubItem to="/docs/guides/sql-queries" currentPath={currentPath}>SQL Queries</SubItem>
```

Insert this line between the existing "Live Queries" SubItem (currently line 176) and "Schema & Type Safety" SubItem (currently line 177) in `DocsSidebar.tsx`. The sidebar is manually curated — SubItem placement controls page ordering, not the frontmatter `order` field.

## Acceptance Criteria

1. `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` exists with all 7 sections listed in R1
2. The page frontmatter has `title: SQL Queries`, `order: 13`
3. The page explains the `datafusion` Cargo feature flag with a build command
4. The page documents `SqlQueryPayload` and `SqlQueryRespPayload` wire message structures
5. The page lists supported SQL clauses: SELECT, WHERE, GROUP BY, ORDER BY, LIMIT, OFFSET, DISTINCT
6. The page includes a data type mapping table (MsgPack -> Arrow/SQL)
7. The page lists all 6 limitations: single-node, no JOINs, no live SQL, schema required, no filter pushdown, cache rebuild
8. The sidebar shows "SQL Queries" link under Guides, between "Live Queries" and "Schema & Type Safety"
9. The page follows existing guide conventions: MDX frontmatter, CodeBlock import, lucide icons import, FeatureCard usage

## Constraints

- DO NOT invent API methods that do not exist -- this documents the wire protocol, not a client SDK method (no `client.sql()` wrapper exists yet)
- DO NOT claim distributed SQL or JOIN support -- these are explicitly listed as limitations
- DO NOT modify any existing guide pages
- Follow the existing MDX conventions exactly (import paths, component usage, code block patterns)

## Assumptions

- The `order` field in frontmatter provides approximate content-collection ordering (multiple guides share the same value, e.g., schema, indexing, and pub-sub all use 13). The actual sidebar position is controlled by SubItem placement in `DocsSidebar.tsx`
- DataFusion's standard SQL dialect documentation (available at datafusion.apache.org) covers the full syntax; this page covers only the subset relevant to TopGun's single-map, single-node usage
- The page uses exported code string variables with the `CodeBlock` component, matching the pattern in existing guides (e.g., `full-text-search.mdx`, `live-queries.mdx`)
- SQL query examples use realistic but fictional data (products, orders, users) consistent with other documentation pages

## Audit History

### Audit v1 (2026-03-25)
**Status:** NEEDS_REVISION

**Context Estimate:** ~10% total

**Delta validation:** 2/2 entries valid

**Critical:**
1. **Frontmatter `order: 13` conflicts with existing `schema.mdx`:** The file `apps/docs-astro/src/content/docs/guides/schema.mdx` already uses `order: 13`. Setting the new SQL Queries page to `order: 13` creates a collision. Either bump `schema.mdx` to `order: 14` (and cascade Full-Text Search to `order: 15`, etc.) or pick a different order value for the new page. Note: this also contradicts the constraint "DO NOT modify any existing guide pages" if bumping schema's order is needed, so the constraint may need relaxation.
2. **Sidebar line numbers are incorrect:** R2 says "line 177" (Live Queries) and "line 178" (Schema). Actual lines are 176 and 177 respectively. Fix the line references to avoid implementer confusion.

**Recommendations:**
3. The assumption "13 places SQL Queries between Live Queries (12) and Full-Text Search (14)" is incorrect -- Schema (13) sits between them. Update the assumption text to reflect the actual ordering situation.
4. The sidebar ordering is manually curated and does not follow frontmatter `order` values (e.g., Full-Text Search has `order: 14` but appears at sidebar line 179, well after Schema at line 177). The spec should clarify that sidebar position is determined by the SubItem placement in `DocsSidebar.tsx`, not by the frontmatter `order` field.

### Response v1 (2026-03-25)
**Applied:** All (2 critical + 2 recommendations)

**Changes:**
1. [✓] Order collision — clarified that `order: 13` is shared by multiple guides (schema, indexing, pub-sub) and that sidebar position is controlled by DocsSidebar.tsx SubItem placement, not frontmatter order
2. [✓] Line numbers — corrected R2 to reference line 176 (Live Queries) and line 177 (Schema), added note that sidebar is manually curated
3. [✓] Assumption text — rewrote to explain that order values are approximate and shared, sidebar controlled by DocsSidebar.tsx
4. [✓] Sidebar mechanism — added explicit clarification in both R2 and frontmatter sections that SubItem placement determines sidebar ordering

### Audit v2 (2026-03-25)
**Status:** APPROVED

**Context Estimate:** ~10% total

**Delta validation:** 2/2 entries valid

**Strategic fit:** Aligned with project goals -- documents an implemented feature that has no user-facing docs.

**Project compliance:** Honors PROJECT.md decisions. This is a docs spec targeting MDX/TSX files, not Rust packages, so language profile constraints do not apply.

**Comment:** Well-structured documentation spec with clear section requirements, concrete acceptance criteria, and appropriate constraints. All issues from Audit v1 have been addressed -- the order collision is properly explained, line numbers are correct, and the sidebar mechanism is clearly documented. Ready for implementation.

---

## Execution Summary

**Executed:** 2026-03-25
**Commits:** 2

### Files Created
- `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` — New guide page covering all 7 required sections: introduction, enabling DataFusion, wire protocol, supported SQL syntax, data type mapping table, 3 realistic example queries, and limitations

### Files Modified
- `apps/docs-astro/src/components/docs/DocsSidebar.tsx` — Added SQL Queries SubItem between Live Queries (line 176) and Schema & Type Safety (line 178)

### Acceptance Criteria Status
- [x] `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` exists with all 7 sections
- [x] Frontmatter has `title: SQL Queries`, `order: 13`
- [x] Page explains the `datafusion` Cargo feature flag with `cargo build -p topgun-server --features datafusion`
- [x] Page documents `SqlQueryPayload` and `SqlQueryRespPayload` wire message structures with field tables
- [x] Page lists supported SQL clauses: SELECT, WHERE, GROUP BY, ORDER BY, LIMIT, OFFSET, DISTINCT
- [x] Page includes data type mapping table (MsgPack -> Arrow/SQL) with all 9 type mappings including `_key` note
- [x] Page lists all 6 limitations: single-node, no JOINs, no live SQL, schema required, no filter pushdown, cache rebuild
- [x] Sidebar shows "SQL Queries" link under Guides, between "Live Queries" and "Schema & Type Safety"
- [x] Page follows existing guide conventions: MDX frontmatter, CodeBlock exports, lucide icon imports, FeatureCard usage, brand token classes

### Deviations
None.

### Notes
- The page intentionally omits a `client.sql()` SDK method per the constraint "DO NOT invent API methods that do not exist" — a callout box explains that the wire protocol must be used directly until a higher-level API is added.
- Wire protocol example uses MsgPack-aware JSON representation for readability; actual encoding is binary MsgPack as noted in the text.
- `HAVING` clause added to SQL syntax table as it is a standard DataFusion-supported clause relevant to GROUP BY queries demonstrated in the examples.

---

## Review History

### Review v1 (2026-03-25)
**Result:** APPROVED
**Reviewer:** impl-reviewer (subagent)

**Findings:**

**Passed:**
- [✓] AC1: `sql-queries.mdx` exists with all 7 sections in order (Introduction, Enabling DataFusion, Wire Protocol, Supported SQL Syntax, Supported Data Types, Example Queries, Limitations)
- [✓] AC2: Frontmatter has `title: SQL Queries`, `order: 13`, and the required `description` field — matches spec exactly
- [✓] AC3: `datafusion` feature flag documented with `cargo build -p topgun-server --features datafusion`; explains that without the flag `SQL_QUERY` returns `error: "SqlNotEnabled"` and that `SchemaProvider` registration is required
- [✓] AC4: `SqlQueryPayload` and `SqlQueryRespPayload` documented as field tables with correct types and descriptions, plus a conceptual JSON example and a working TypeScript WebSocket client example
- [✓] AC5: All required clauses documented in the clauses table: SELECT, WHERE (full operator list), GROUP BY, ORDER BY (ASC/DESC), LIMIT/OFFSET, DISTINCT; `HAVING` added as an additional supported clause (correct and useful)
- [✓] AC6: Data type mapping table covers all 9 required rows (Integer/BIGINT, Float/DOUBLE, String/VARCHAR, Boolean/BOOLEAN, Binary/BYTEA, Integer-millis/TIMESTAMP, Array/LIST, Nil/NULL, Complex-Map/VARCHAR) plus the `_key` synthetic column note
- [✓] AC7: All 6 limitations listed verbatim with accurate descriptions; two of them are also surfaced in FeatureCards for visual emphasis
- [✓] AC8: `DocsSidebar.tsx` line 177 has `<SubItem to="/docs/guides/sql-queries" currentPath={currentPath}>SQL Queries</SubItem>` between Live Queries (line 176) and Schema & Type Safety (line 178)
- [✓] AC9: Follows existing conventions — MDX frontmatter, exported code string variables, `CodeBlock` import from correct relative path, lucide icon imports, `FeatureCard` component usage, brand token CSS classes; icon colors (`text-brand-subtle`, `text-green-500`, `text-orange-500`) match pattern established in `live-queries.mdx`, `ttl.mdx`, and `authentication.mdx`
- [✓] Constraint compliance: No `client.sql()` method invented; a callout box explicitly states this is not available and directs users to use the wire protocol directly
- [✓] Constraint compliance: No distributed SQL or JOIN support claimed; both are listed as limitations
- [✓] Constraint compliance: No existing guide pages were modified
- [✓] Architecture: Breadcrumb navigation and prev/next page links correctly point to `/docs/guides/live-queries` (previous) and `/docs/guides/schema` (next), consistent with sidebar ordering
- [✓] Code quality: Example queries are realistic, use correct fictional data sets (products/orders/users), and demonstrate the full range of documented syntax

**Summary:** The implementation fully satisfies all 9 acceptance criteria. The page content is accurate, well-structured, and consistently follows the conventions established in existing guide pages. No issues found.

---

## Completion

**Completed:** 2026-03-25
**Total Commits:** 2
**Review Cycles:** 1

### Outcome

Added a comprehensive DataFusion SQL query documentation page covering all 7 required sections (introduction, enabling, wire protocol, syntax, data types, examples, limitations) and integrated it into the docs sidebar.

### Key Files

- `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` — SQL query guide with wire protocol docs, syntax reference, data type mapping, and limitations
- `apps/docs-astro/src/components/docs/DocsSidebar.tsx` — Sidebar entry for SQL Queries page

### Changes Applied

**Added:**
- `apps/docs-astro/src/content/docs/guides/sql-queries.mdx` — New guide page covering DataFusion SQL query feature

**Modified:**
- `apps/docs-astro/src/components/docs/DocsSidebar.tsx` — Added SQL Queries SubItem between Live Queries and Schema & Type Safety

### Patterns Established

None — followed existing patterns.

### Spec Deviations

None — implemented as specified.
