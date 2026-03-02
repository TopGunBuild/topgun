# Phase 5: Multi-Tenancy & Cloud Platform - Technical Specification

**Version:** 1.0
**Date:** 2025-12-23
**Status:** Draft
**Dependencies:** Phase 4 complete (50K+ ops/sec cluster)

---

## Executive Summary

Phase 5 transforms TopGun from a self-hosted cluster into a cloud-ready multi-tenant platform capable of serving thousands of isolated tenants with usage-based billing, rate limiting, and operational tooling.

### Current State (Post-Phase 4)
- Cluster: 50,000+ ops/sec
- Single tenant per deployment
- No usage tracking
- No rate limiting
- Manual operations

### Target State (Phase 5)
- Multi-tenant data isolation
- Per-tenant rate limiting and quotas
- Usage metering for billing
- Tenant management API
- Operational dashboard ready

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      MULTI-TENANT ARCHITECTURE                           │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                        API Gateway Layer                            │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │ │
│  │  │ Auth/Tenant  │  │ Rate Limiter │  │ Usage Meter              │ │ │
│  │  │ Resolution   │  │ (per-tenant) │  │ (ops, storage, conn)     │ │ │
│  │  └──────────────┘  └──────────────┘  └──────────────────────────┘ │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                    │                                     │
│  ┌─────────────────────────────────┴──────────────────────────────────┐ │
│  │                      Tenant Isolation Layer                         │ │
│  │                                                                      │ │
│  │   Tenant A (ns: tenant_a/)    Tenant B (ns: tenant_b/)             │ │
│  │   ┌─────────────────────┐     ┌─────────────────────┐              │ │
│  │   │ Maps: users, todos  │     │ Maps: products, orders │           │ │
│  │   │ Quota: 10K ops/sec  │     │ Quota: 50K ops/sec    │           │ │
│  │   │ Storage: 1GB        │     │ Storage: 10GB         │           │ │
│  │   └─────────────────────┘     └─────────────────────────┘          │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                    │                                     │
│  ┌─────────────────────────────────┴──────────────────────────────────┐ │
│  │                      TopGun Cluster (Phase 4)                       │ │
│  │   ┌─────────┐     ┌─────────┐     ┌─────────┐                      │ │
│  │   │ Node A  │◄───►│ Node B  │◄───►│ Node C  │                      │ │
│  │   └─────────┘     └─────────┘     └─────────┘                      │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Performance Targets

| Metric | Phase 4 | Phase 5 Target | Notes |
|--------|---------|----------------|-------|
| **Throughput** | 50K ops/sec | 50K ops/sec | Maintained |
| **Tenants** | 1 | 10,000+ | Per cluster |
| **Isolation overhead** | N/A | <5% | Namespace lookup |
| **Rate limit check** | N/A | <0.1ms | In-memory |
| **Meter overhead** | N/A | <1% | Async batched |

---

## Components

### 5.1 Tenant Isolation

**Goal:** Complete data isolation between tenants using namespace prefixing.

**Approach:** Transparent namespace prefixing at the server layer.

```typescript
// Client sees:
client.getMap('users').set('user-1', data);

// Server stores as:
// Key: "tenant_abc123/users/user-1"
// Partition: hash("tenant_abc123/users/user-1") % 271
```

**Benefits:**
- Zero client-side changes
- Existing CRDT logic unchanged
- Partition distribution across tenants
- Simple backup/restore per tenant

### 5.2 Rate Limiting

**Goal:** Per-tenant rate limiting with graceful degradation.

**Limits:**
| Resource | Free Tier | Pro Tier | Enterprise |
|----------|-----------|----------|------------|
| Ops/sec | 100 | 10,000 | Custom |
| Connections | 10 | 1,000 | Custom |
| Maps | 10 | 100 | Unlimited |
| Storage | 100MB | 10GB | Custom |

**Algorithm:** Token bucket with sliding window.

### 5.3 Usage Metering

**Goal:** Track usage for billing with minimal overhead.

**Metrics:**
- Operations (read/write count)
- Storage (bytes)
- Connection-hours
- Bandwidth (in/out)

**Approach:** In-memory counters with periodic flush to TimescaleDB/ClickHouse.

### 5.4 Tenant Management API

**Goal:** CRUD operations for tenant lifecycle.

**Endpoints:**
```
POST   /api/tenants              # Create tenant
GET    /api/tenants/:id          # Get tenant
PATCH  /api/tenants/:id          # Update tenant (quotas, status)
DELETE /api/tenants/:id          # Delete tenant (soft delete)
GET    /api/tenants/:id/usage    # Get usage metrics
POST   /api/tenants/:id/keys     # Generate API key
```

### 5.5 Operational Tooling

**Goal:** Tools for operating multi-tenant cluster.

**Features:**
- Tenant migration between nodes
- Tenant data export/import
- Audit logging
- Usage dashboard data API

---

## Task Breakdown

| Task | File | Description | Effort |
|------|------|-------------|--------|
| 5.1 | PHASE_5_01_RESEARCH.md | Research multi-tenancy patterns | 2-3h |
| 5.2 | PHASE_5_02_TENANT_ISOLATION.md | Namespace prefixing, TenantContext | 6-8h |
| 5.3 | PHASE_5_03_RATE_LIMITING.md | RateLimiter, quota enforcement | 4-6h |
| 5.4 | PHASE_5_04_USAGE_METERING.md | UsageMeter, async flush, storage | 6-8h |
| 5.5 | PHASE_5_05_TENANT_API.md | REST API, tenant lifecycle | 4-6h |
| 5.6 | PHASE_5_06_OPERATIONAL.md | Migration, backup, audit | 4-6h |
| 5.7 | PHASE_5_07_INTEGRATION.md | Integration & configuration | 4-6h |
| 5.8 | PHASE_5_08_BENCHMARKS.md | Multi-tenant benchmarks | 4-6h |

**Total estimated effort:** 35-50 hours

---

## Key Files to Create/Modify

### New Files (Server Package)

```
packages/server/src/tenant/
├── TenantContext.ts          # Tenant resolution and context
├── TenantManager.ts          # Tenant CRUD operations
├── TenantIsolation.ts        # Namespace prefixing logic
├── RateLimiter.ts            # Token bucket rate limiting
├── QuotaManager.ts           # Quota tracking and enforcement
├── UsageMeter.ts             # Usage tracking
├── UsageStore.ts             # Usage persistence (TimescaleDB)
├── TenantRouter.ts           # Express router for tenant API
└── __tests__/
    ├── TenantIsolation.test.ts
    ├── RateLimiter.test.ts
    └── UsageMeter.test.ts
```

### Modified Files

```
packages/server/src/ServerCoordinator.ts  # Add tenant resolution
packages/server/src/handlers/*.ts          # Add tenant context
packages/core/src/types/                   # Add tenant types
packages/client/src/TopGun.ts              # Add tenant ID config
```

---

## Detailed Component Specs

### 5.2 Tenant Isolation

```typescript
interface TenantContext {
  tenantId: string;
  plan: 'free' | 'pro' | 'enterprise';
  quotas: TenantQuotas;
  metadata: Record<string, unknown>;
}

interface TenantQuotas {
  maxOpsPerSecond: number;
  maxConnections: number;
  maxMaps: number;
  maxStorageBytes: number;
}

class TenantIsolation {
  // Prefix key with tenant namespace
  prefixKey(tenantId: string, mapName: string, key: string): string {
    return `t:${tenantId}/${mapName}/${key}`;
  }

  // Extract tenant from prefixed key
  extractTenant(prefixedKey: string): { tenantId: string; mapName: string; key: string } {
    const match = prefixedKey.match(/^t:([^/]+)\/([^/]+)\/(.+)$/);
    if (!match) throw new Error('Invalid prefixed key');
    return { tenantId: match[1], mapName: match[2], key: match[3] };
  }

  // Validate tenant can access key
  validateAccess(tenantContext: TenantContext, prefixedKey: string): boolean {
    const { tenantId } = this.extractTenant(prefixedKey);
    return tenantContext.tenantId === tenantId;
  }
}
```

### 5.3 Rate Limiting

```typescript
interface RateLimitResult {
  allowed: boolean;
  remaining: number;
  resetAt: number;
  retryAfter?: number;
}

class RateLimiter {
  private buckets: Map<string, TokenBucket>;

  constructor(private config: RateLimiterConfig) {
    this.buckets = new Map();
  }

  // Check and consume tokens
  checkLimit(tenantId: string, tokens: number = 1): RateLimitResult {
    const bucket = this.getOrCreateBucket(tenantId);
    return bucket.tryConsume(tokens);
  }

  // Update tenant quota (runtime)
  updateQuota(tenantId: string, newOpsPerSecond: number): void {
    const bucket = this.buckets.get(tenantId);
    if (bucket) {
      bucket.updateRate(newOpsPerSecond);
    }
  }

  // Get current usage
  getUsage(tenantId: string): { current: number; limit: number; percentage: number } {
    const bucket = this.buckets.get(tenantId);
    if (!bucket) return { current: 0, limit: 0, percentage: 0 };
    return bucket.getUsage();
  }
}

class TokenBucket {
  private tokens: number;
  private lastRefill: number;
  private rate: number;      // tokens per second
  private capacity: number;  // max burst

  tryConsume(count: number): RateLimitResult {
    this.refill();

    if (this.tokens >= count) {
      this.tokens -= count;
      return {
        allowed: true,
        remaining: Math.floor(this.tokens),
        resetAt: Date.now() + 1000,
      };
    }

    return {
      allowed: false,
      remaining: 0,
      resetAt: Date.now() + 1000,
      retryAfter: Math.ceil((count - this.tokens) / this.rate * 1000),
    };
  }

  private refill(): void {
    const now = Date.now();
    const elapsed = (now - this.lastRefill) / 1000;
    this.tokens = Math.min(this.capacity, this.tokens + elapsed * this.rate);
    this.lastRefill = now;
  }
}
```

### 5.4 Usage Metering

```typescript
interface UsageMetrics {
  tenantId: string;
  timestamp: number;
  period: 'minute' | 'hour' | 'day';

  // Operations
  readOps: number;
  writeOps: number;

  // Storage
  storageBytes: number;
  storageDelta: number;

  // Connections
  connectionMinutes: number;
  peakConnections: number;

  // Bandwidth
  bytesIn: number;
  bytesOut: number;
}

class UsageMeter {
  private counters: Map<string, TenantCounters>;
  private flushInterval: NodeJS.Timeout;

  constructor(
    private store: UsageStore,
    private config: UsageMeterConfig
  ) {
    this.counters = new Map();
    this.startFlushLoop();
  }

  // Record operation (called on every op)
  recordOp(tenantId: string, type: 'read' | 'write', bytes: number): void {
    const counters = this.getOrCreateCounters(tenantId);
    if (type === 'read') {
      counters.readOps++;
      counters.bytesOut += bytes;
    } else {
      counters.writeOps++;
      counters.bytesIn += bytes;
    }
  }

  // Record connection event
  recordConnection(tenantId: string, event: 'open' | 'close'): void {
    const counters = this.getOrCreateCounters(tenantId);
    if (event === 'open') {
      counters.activeConnections++;
      counters.peakConnections = Math.max(
        counters.peakConnections,
        counters.activeConnections
      );
    } else {
      counters.activeConnections--;
    }
  }

  // Flush to persistent storage (async, batched)
  private async flush(): Promise<void> {
    const timestamp = Date.now();
    const metrics: UsageMetrics[] = [];

    for (const [tenantId, counters] of this.counters) {
      metrics.push({
        tenantId,
        timestamp,
        period: 'minute',
        readOps: counters.readOps,
        writeOps: counters.writeOps,
        storageBytes: counters.storageBytes,
        storageDelta: counters.storageDelta,
        connectionMinutes: counters.connectionMinutes,
        peakConnections: counters.peakConnections,
        bytesIn: counters.bytesIn,
        bytesOut: counters.bytesOut,
      });

      // Reset counters
      counters.reset();
    }

    await this.store.batchInsert(metrics);
  }

  private startFlushLoop(): void {
    this.flushInterval = setInterval(
      () => this.flush(),
      this.config.flushIntervalMs // Default: 60000 (1 minute)
    );
  }
}
```

### 5.5 Tenant Management API

```typescript
// Express router for tenant management
const tenantRouter = Router();

// Create tenant
tenantRouter.post('/tenants', async (req, res) => {
  const { name, plan, quotas, metadata } = req.body;

  const tenant = await tenantManager.create({
    name,
    plan: plan || 'free',
    quotas: quotas || DEFAULT_QUOTAS[plan],
    metadata,
  });

  res.status(201).json(tenant);
});

// Get tenant
tenantRouter.get('/tenants/:id', async (req, res) => {
  const tenant = await tenantManager.get(req.params.id);
  if (!tenant) return res.status(404).json({ error: 'Tenant not found' });
  res.json(tenant);
});

// Update tenant
tenantRouter.patch('/tenants/:id', async (req, res) => {
  const { quotas, status, metadata } = req.body;

  const tenant = await tenantManager.update(req.params.id, {
    quotas,
    status,
    metadata,
  });

  res.json(tenant);
});

// Get tenant usage
tenantRouter.get('/tenants/:id/usage', async (req, res) => {
  const { period = 'day', from, to } = req.query;

  const usage = await usageMeter.getUsage(req.params.id, {
    period,
    from: from ? new Date(from) : undefined,
    to: to ? new Date(to) : undefined,
  });

  res.json(usage);
});

// Generate API key
tenantRouter.post('/tenants/:id/keys', async (req, res) => {
  const { name, scopes } = req.body;

  const apiKey = await tenantManager.generateApiKey(req.params.id, {
    name,
    scopes: scopes || ['read', 'write'],
  });

  res.status(201).json(apiKey);
});
```

---

## Message Flow with Multi-Tenancy

```
Client Request:
┌─────────────────────────────────────────────────────────────────┐
│  1. WebSocket message arrives                                    │
│  2. Extract tenant ID from JWT or API key                       │
│  3. Load TenantContext (cached)                                 │
│  4. Check rate limit → reject if exceeded                       │
│  5. Prefix keys with tenant namespace                           │
│  6. Process operation (existing logic)                          │
│  7. Record usage metrics (async)                                │
│  8. Send response                                               │
└─────────────────────────────────────────────────────────────────┘
```

```typescript
// In ServerCoordinator.handleMessage()
async handleMessage(client: Client, message: Message): Promise<void> {
  // 1. Get tenant context (cached)
  const tenant = await this.tenantResolver.resolve(client);

  // 2. Check rate limit
  const rateLimit = this.rateLimiter.checkLimit(tenant.id, 1);
  if (!rateLimit.allowed) {
    return this.sendError(client, {
      code: 'RATE_LIMITED',
      retryAfter: rateLimit.retryAfter,
    });
  }

  // 3. Prefix keys in message
  const prefixedMessage = this.tenantIsolation.prefixMessage(tenant.id, message);

  // 4. Process (existing logic)
  await this.processMessage(client, prefixedMessage);

  // 5. Record usage (async, non-blocking)
  this.usageMeter.recordOp(tenant.id, message.type, message.size);
}
```

---

## Database Schema (Usage Storage)

```sql
-- TimescaleDB hypertable for usage metrics
CREATE TABLE usage_metrics (
  time        TIMESTAMPTZ NOT NULL,
  tenant_id   TEXT NOT NULL,
  period      TEXT NOT NULL,  -- 'minute', 'hour', 'day'

  read_ops    BIGINT DEFAULT 0,
  write_ops   BIGINT DEFAULT 0,

  storage_bytes    BIGINT DEFAULT 0,
  storage_delta    BIGINT DEFAULT 0,

  connection_minutes  INTEGER DEFAULT 0,
  peak_connections    INTEGER DEFAULT 0,

  bytes_in    BIGINT DEFAULT 0,
  bytes_out   BIGINT DEFAULT 0
);

SELECT create_hypertable('usage_metrics', 'time');

-- Index for tenant queries
CREATE INDEX idx_usage_tenant_time ON usage_metrics (tenant_id, time DESC);

-- Continuous aggregate for hourly rollups
CREATE MATERIALIZED VIEW usage_hourly
WITH (timescaledb.continuous) AS
SELECT
  time_bucket('1 hour', time) AS hour,
  tenant_id,
  SUM(read_ops) AS read_ops,
  SUM(write_ops) AS write_ops,
  MAX(storage_bytes) AS storage_bytes,
  SUM(connection_minutes) AS connection_minutes,
  MAX(peak_connections) AS peak_connections,
  SUM(bytes_in) AS bytes_in,
  SUM(bytes_out) AS bytes_out
FROM usage_metrics
WHERE period = 'minute'
GROUP BY hour, tenant_id;

-- Tenants table
CREATE TABLE tenants (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  plan        TEXT NOT NULL DEFAULT 'free',
  status      TEXT NOT NULL DEFAULT 'active',

  quota_ops_per_second    INTEGER NOT NULL,
  quota_connections       INTEGER NOT NULL,
  quota_maps              INTEGER NOT NULL,
  quota_storage_bytes     BIGINT NOT NULL,

  metadata    JSONB DEFAULT '{}',

  created_at  TIMESTAMPTZ DEFAULT NOW(),
  updated_at  TIMESTAMPTZ DEFAULT NOW()
);

-- API keys table
CREATE TABLE api_keys (
  id          TEXT PRIMARY KEY,
  tenant_id   TEXT NOT NULL REFERENCES tenants(id),
  name        TEXT NOT NULL,
  key_hash    TEXT NOT NULL,  -- bcrypt hash
  scopes      TEXT[] NOT NULL DEFAULT ARRAY['read', 'write'],

  last_used_at  TIMESTAMPTZ,
  expires_at    TIMESTAMPTZ,

  created_at  TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_api_keys_tenant ON api_keys (tenant_id);
```

---

## Client SDK Changes

```typescript
// New client configuration
const client = new TopGunClient({
  serverUrl: 'wss://api.topgun.cloud',

  // Authentication (one of):
  apiKey: 'tg_live_abc123...',        // API key
  // OR
  jwt: 'eyJ...',                       // JWT with tenant claim

  // Optional tenant override (for admin tools)
  tenantId: 'tenant_abc123',
});
```

---

## Execution Order

```
#01 Research
    ↓
#02 Tenant Isolation (namespace prefixing)
    ↓
#03 Rate Limiting ──────────┐
    ↓                       │
#04 Usage Metering          │
    ↓                       │
#05 Tenant API ─────────────┤
    ↓                       │
#06 Operational (backup, audit)
    ↓                       │
#07 Integration ────────────┘
    ↓
#08 Benchmarks
```

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Namespace collision | High | UUID tenant IDs, validation |
| Rate limiter memory | Medium | LRU eviction, Redis option |
| Usage meter data loss | Medium | WAL, batch with retry |
| Cross-tenant data leak | Critical | Unit tests, audit logging |
| Performance overhead | Medium | Benchmark, optimize hot paths |

---

## Success Criteria

1. **Isolation:** Zero cross-tenant data access
2. **Rate limiting:** <0.1ms overhead per operation
3. **Metering:** <1% throughput impact
4. **API:** Full tenant lifecycle management
5. **Throughput:** Maintain 50K ops/sec with 1000 tenants
6. **Tests:** 100% coverage on isolation logic

---

## Billing Integration

Phase 5 provides usage data. Billing integration options:

1. **Stripe Metered Billing**
   - Push usage to Stripe via Usage Records API
   - Monthly invoice generation

2. **Custom Billing**
   - Query usage_metrics table
   - Generate invoices via custom logic

3. **Usage Webhook**
   - Push usage events to external billing system
   - Real-time billing updates

```typescript
// Example: Stripe integration
class StripeBillingAdapter {
  async reportUsage(tenantId: string, period: Date): Promise<void> {
    const usage = await usageMeter.getUsage(tenantId, {
      period: 'day',
      from: startOfDay(period),
      to: endOfDay(period),
    });

    await stripe.subscriptionItems.createUsageRecord(
      tenant.stripeSubscriptionItemId,
      {
        quantity: usage.writeOps + usage.readOps,
        timestamp: Math.floor(period.getTime() / 1000),
        action: 'set',
      }
    );
  }
}
```

---

## References

- [Stripe Metered Billing](https://stripe.com/docs/billing/subscriptions/usage-based)
- [TimescaleDB Continuous Aggregates](https://docs.timescale.com/timescaledb/latest/how-to-guides/continuous-aggregates/)
- [Token Bucket Algorithm](https://en.wikipedia.org/wiki/Token_bucket)
- [Multi-tenant SaaS Patterns](https://docs.microsoft.com/en-us/azure/architecture/guide/multitenant/overview)
