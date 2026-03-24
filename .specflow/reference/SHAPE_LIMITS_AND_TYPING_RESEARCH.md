# Shape Subscription: Limits and Type Safety Research

> Research ID: RES-004 | Created: 2026-03-22

## 1. Comparison: Limits and Pagination

| Project | Default Limit | Server-Side Max | What Happens If Too Much Data | Pagination Support |
|---------|--------------|-----------------|-------------------------------|-------------------|
| **ElectricSQL** | None | None documented | All matching rows returned. Chunked shape logs handle transport. POST for large parameter sets. | `requestSnapshot()` with `limit`/`offset`/`orderBy` (not on main subscription) |
| **PowerSync** | None per bucket | 1,000 buckets per user | Bucket count limit rejects excess subscriptions. No per-bucket row cap. | N/A (sync rules declaratively filter) |
| **Replicache** | None | Browser-limited (~100s MB) | Pull diffs keep incremental sync small. Initial sync can be large. | Pull returns diffs (patches), not paginated results |
| **Firestore** | None (all matching docs) | ~1MB per doc. Previously had 100K/query cap (removed). | Returns all matching docs. Billing acts as natural brake. | `limit()`, `limitToLast()`, cursor-based (`startAt`/`endAt`) |
| **Firebase RTDB** | None | 75M nodes per listener path. 16MB per write. | Hard cap at 75M nodes. Bandwidth limits. | `limitToFirst(n)`, `limitToLast(n)` |
| **Hazelcast Near Cache** | MAX_INT (on-heap), 10,000 (native) | Configurable per cache | Eviction kicks in (LRU default). Old entries removed. | N/A (cache eviction, not query pagination) |
| **LiveStore** | N/A | N/A (local SQLite) | Standard SQLite limits apply | SQL `LIMIT`/`OFFSET` |

### Key Takeaway

No offline-first sync project enforces a default row limit on subscriptions. They all rely on the developer to scope data via filters/sync rules. However, TopGun is a data grid (not just a sync engine) and serves potentially malicious or buggy clients, making server-side protection essential. Hazelcast's 10,000-entry default for native near caches is the closest precedent.

## 2. Comparison: Type Safety

| Project | Filter Typing | Field Projection Typing | Schema Source | Codegen Required | DX Rating |
|---------|--------------|------------------------|--------------|-----------------|-----------|
| **ElectricSQL** | String SQL (`where?: string`) - loose | `columns?: string[]` - loose | Generic `<T>` on output only | No | Low |
| **PowerSync** | Delegated to ORM (Kysely/Drizzle) | Delegated to ORM | TS schema definition + ORM | No | Medium (via ORM) |
| **Replicache** | Untyped function `(tx) => tx.scan()` | N/A | Mutator types only | No | Low for reads |
| **Prisma** | Codegen `UserWhereInput` objects - strong | `select: { field: true }` narrows return type | `.prisma` schema file | Yes (`prisma generate`) | Excellent |
| **Drizzle** | Column references `eq(users.email, x)` - strong | Object select `{ id: users.id }` - strong | Schema-as-TS-code | No | Excellent |
| **tRPC** | Zod input validation - strong (for RPC) | N/A (RPC output typing) | Zod schemas | No | Strong for RPC |
| **TopGun (current)** | `PredicateNode` tree with `string` attributes - loose | `fields?: string[]` - loose | None | No | Low |

### Key Takeaway

The industry splits into two camps: (1) string-based/untyped filters (ElectricSQL, Replicache) that are easy to implement but error-prone, and (2) schema-derived typed filters (Prisma, Drizzle) that catch errors at compile time. TopGun's planned Zod-first schema strategy positions it well for approach (2), but this requires the schema system (TODO-069) to land first.

## 3. Recommendations for TopGun

### Limits: Server-Side Configurable Max (Default 10,000)

**What:**
- Add `max_shape_records: u32` to server config (default: 10,000)
- Server clamps client-requested `limit` to `min(requested, max_shape_records)`
- When client omits `limit`, server applies `max_shape_records` as ceiling
- Set `has_more: true` in `ShapeRespPayload` when total matches exceed delivered records
- Log warning when clamping occurs

**Why:**
- Prevents runaway subscriptions (e.g., `subscribeShape("events")` on a million-row map)
- `has_more` already exists in the wire protocol - no protocol changes needed
- 10,000 is generous for CRUD apps, matches Hazelcast NATIVE near-cache default
- Server-side enforcement protects against malicious/buggy clients

**Future (not now):**
- Cursor-based shape pagination (`SHAPE_PAGE_REQ`/`SHAPE_PAGE_RESP`) for clients that need to page through large result sets
- Per-map configurable limits via `MapSchema` settings

### Type Safety: Two-Phase Approach

#### Phase A: Generic Output Typing (do now, low effort)

Add a type parameter to `ShapeHandle` and `subscribeShape`:

```typescript
// Before
subscribeShape(mapName: string, options?: ShapeSubscribeOptions): ShapeHandle

// After
subscribeShape<T = Record<string, unknown>>(
  mapName: string,
  options?: ShapeSubscribeOptions
): ShapeHandle<T>

// ShapeHandle<T>.records: Map<string, T>  (was Map<string, any>)
// ShapeUpdate<T>.value: T | undefined      (was any)
```

This follows ElectricSQL's pattern. No runtime changes. Immediate autocomplete benefits.

#### Phase B: Schema-Derived Filter Typing (after TODO-069 schema system)

Once Zod schemas are registered per map, derive filter types:

```typescript
// From Zod schema:
const UserSchema = z.object({
  id: z.string(),
  name: z.string(),
  status: z.enum(['active', 'inactive']),
  age: z.number(),
});

// Generate (or infer) filter type:
type UserFilter = {
  id?: string | { eq?: string; in?: string[] };
  name?: string | { eq?: string; contains?: string };
  status?: 'active' | 'inactive' | { eq?: string; in?: string[] };
  age?: number | { gt?: number; lt?: number; gte?: number; lte?: number };
};

// Type-safe field projection:
type UserFields = keyof z.infer<typeof UserSchema>;

// Usage:
client.subscribeShape<User>('users', {
  filter: { status: 'active', age: { gte: 18 } },  // Type-checked
  fields: ['name', 'email'],                         // Type-checked
  limit: 100,
});
```

The typed filter object compiles to `PredicateNode` at runtime. Wire protocol unchanged.

**Pattern precedent:** Drizzle's schema-as-code is the closest match to TopGun's Zod-first strategy. Prisma-style codegen is an alternative but adds a build step.

### Priority

1. **Limits (server config)** - Small change, high safety impact. Do in next shape-related spec.
2. **Phase A typing** - Small change, high DX impact. Can be done independently.
3. **Phase B typing** - Depends on schema system (TODO-069). Plan for v2.0.

## References

- [ElectricSQL Shapes](https://electric-sql.com/docs/guides/shapes) | [TS Client](https://electric-sql.com/docs/api/clients/typescript)
- [PowerSync Sync Streams](https://docs.powersync.com/sync/streams/overview) | [Sync Rules](https://docs.powersync.com/usage/sync-rules)
- [Replicache How It Works](https://doc.replicache.dev/concepts/how-it-works)
- [Firestore Limits](https://firebase.google.com/docs/firestore/quotas) | [Order/Limit](https://firebase.google.com/docs/firestore/query-data/order-limit-data)
- [Hazelcast Near Cache](https://docs.hazelcast.com/hazelcast/5.3/performance/near-cache)
- [Prisma Type Safety](https://www.prisma.io/docs/orm/prisma-client/type-safety)
- [Drizzle Filters](https://orm.drizzle.team/docs/operators) | [Select](https://orm.drizzle.team/docs/select)
- [LiveStore](https://livestore.dev/)
