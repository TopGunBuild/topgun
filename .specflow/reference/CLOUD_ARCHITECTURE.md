# TopGun Cloud — Архитектура и план запуска

> **Дата:** 2026-03-22
> **Статус:** Draft v2 (stress-tested через 5 итераций вопросов)
> **Зрелость:** Средняя — ценообразование валидировано vs конкуренты, архитектура обоснована, но не реализована
> **Зависимости:** TODO-163 (P0 Security), TODO-136 (Rate Limits), TODO-141 (Docker), TODO-033a (LRU Evictor)

---

## 1. Модель хостинга

### Выбор: Shared Instance (Вариант C)

Все пользователи (free + paid) на одном кластере с namespace-level изоляцией. Dedicated instances только для Enterprise ($299+) по запросу.

**Почему не single-tenant (Вариант A):**
- 50 free users после Show HN = 50 контейнеров × $14 = $700/мес без дохода
- Operational overhead: мониторинг, обновления, балансировка N контейнеров
- Для соло-основателя неуправляемо

**Почему не full multi-tenancy (TODO-041):**
- 4-6 недель разработки, отнесено к v3.0
- Для <100 клиентов namespace isolation достаточно

**Что нужно реализовать (мини-изоляция, не TODO-041):**
- Map name prefix: `tenant_abc:todos` вместо `todos`
- Per-connection rate limits (TODO-136)
- Per-connection memory quota через `max_entry_count`
- Shared PostgreSQL с `tenant_id` column
- ~3-5 дней работы поверх TODO-136

### Архитектура

```
Клиенты (browser/mobile)
    │
    │ WSS (TLS обязателен — TODO-163)
    │
    ▼
┌─────────────────────────────────┐
│  Reverse Proxy (Caddy/nginx)    │
│  - TLS termination              │
│  - Tenant routing по subdomain  │
│  - Rate limiting (L7)           │
└─────────┬───────────────────────┘
          │
          ▼
┌─────────────────────────────────┐
│  TopGun Server (Rust, axum)     │
│  - Namespace isolation          │
│  - Per-tenant quotas            │
│  - LRU evictor (TODO-033a)      │
│  - JWT auth (TODO-163 fixes)    │
│  - WebSocket handler            │
│  - 271 partitions               │
└─────────┬───────────────────────┘
          │
          ▼
┌─────────────────────────────────┐
│  PostgreSQL (shared)            │
│  - tenant_id column             │
│  - Cold data fallback           │
│  - sslmode=require              │
└─────────────────────────────────┘
```

### Миграция Free → Paid

**При shared instance модели — миграции НЕТ.** Все на одном кластере. Upgrade = изменение лимитов в tenant config (connections, storage quota, rate limits). Zero downtime, zero code change для клиента.

Если Enterprise хочет dedicated instance — тогда export/import + DNS switch через reverse proxy. Прозрачно для клиента.

---

## 2. Ценообразование (v2, после сравнения с конкурентами)

### Рыночный контекст

| Конкурент | First Paid | Connections | Позиционирование |
|-----------|-----------|-------------|-----------------|
| Supabase Pro | $25/мес | 500 Realtime | "Firebase alternative" |
| Convex Pro | $25/мес | function-based | "Reactive backend" |
| Turso Scaler | $29/мес | N/A (HTTP) | "Edge SQLite" |
| Neon Launch | $19/мес | ~10K pooled | "Serverless Postgres" |
| PowerSync Pro | ~$49/мес | ~500-1000 | "Postgres sync" |
| Firebase Blaze | pay-per-use | 200K | "Google backend" |

**Ключевой инсайт:** $25/мес — рыночная норма для первого paid tier. $99 Starter из первого драфта был в 4x дороже рынка.

### Тарифы TopGun Cloud

| Тариф | Цена | Connections | Storage | Проекты | Целевой клиент |
|-------|------|------------|---------|---------|---------------|
| **Free** | $0 | 100 concurrent | 500MB | 1 | Оценка, хобби, прототипы |
| **Pro** | $25/мес | 1,000 concurrent | 5GB | 5 | Инди/стартапы с production app |
| **Team** | $79/мес | 5,000 concurrent | 25GB | 20 | Растущие команды |
| **Enterprise** | $299+/мес | Unlimited | Unlimited | Unlimited | SLA, dedicated, VPC |

**Позиционирование vs Supabase Pro ($25):** За ту же цену — 2x connections (1000 vs 500) + offline-first + CRDT conflict resolution. Чёткий differentiator.

### Психология ценообразования

- **$0 → $1** — самый сложный переход (credit card barrier). Free tier обязателен
- **$25/мес** — "default acceptable", не требует одобрения руководства
- **$79/мес** — "serious startup", оправдан при ощутимом usage
- **$299/мес** — порог enterprise evaluation, но ниже чем конкуренты ($999)
- Предсказуемая цена > pay-per-use (Firebase billing horror stories — конкурентное преимущество)

---

## 3. Юнит-экономика (v2)

### Per-tenant cost при shared instance

| Ресурс | Free tenant | Pro tenant | Team tenant |
|--------|------------|------------|-------------|
| RAM (amortized) | ~50MB = $0.10 | ~500MB = $0.95 | ~2GB = $3.80 |
| PostgreSQL (shared) | $0.10 | $0.50 | $2.00 |
| Bandwidth | $0.05 | $0.30 | $1.00 |
| **Итого** | **~$0.25/мес** | **~$1.75/мес** | **~$6.80/мес** |

### Break-even и маржинальность

**Инфраструктура:** 1x Hetzner CCX33 (32GB, €55/~$60/мес) + managed PostgreSQL ($25/мес) = **$85/мес base cost**

| Сценарий | Revenue | Infra | Paddle (5%) | Margin |
|----------|---------|-------|-------------|--------|
| 50 Free + 3 Pro | $75/мес | $85 | $3.75 | **-15%** (pre-profit) |
| 50 Free + 5 Pro | $125/мес | $85 | $6.25 | **27%** |
| 30 Free + 10 Pro + 2 Team | $408/мес | $85 | $20.40 | **74%** |
| 30 Free + 20 Pro + 5 Team | $895/мес | $85 | $44.75 | **85%** |
| 20 Free + 30 Pro + 10 Team + 2 Ent | $2,148/мес | $120 | $107.40 | **89%** |

**Break-even: ~4 Pro клиента** ($100 revenue > $85 infra + $5 Paddle)

**Реалистичная прогрессия:**
- Month 1-3 (post-launch): 50 free, 3-5 pro → **$75-125/мес, pre-profit**
- Month 4-6: 80 free, 10-15 pro, 1-2 team → **$330-530/мес, 50-60% margin**
- Month 7-12: 100+ free, 20-30 pro, 5 team → **$895-1,145/мес, 80-85% margin**

**Честная оценка:** Первые 3 месяца будут убыточными ($85/мес из кармана). Это нормальная инвестиция в рост. Free tier — маркетинговый расход.

### RAM capacity при shared instance (CCX33, 32GB)

| Use Case | Bytes/Record | Usable RAM (28GB) | Max Records |
|----------|-------------|-------------------|-------------|
| Todo items | ~512B | 28GB | 54.7M |
| Chat messages | ~812B | 28GB | 34.5M |
| User profiles | ~1.3KB | 28GB | 21.0M |

При 60 tenants (50 free × 50MB + 10 paid × 500MB = 7.5GB) — используется **27% RAM**. Запас огромный.

---

## 4. Технические prerequisites

### Блокеры для Cloud Launch (в порядке приоритета)

| # | TODO | Что | Усилия | Статус |
|---|------|-----|--------|--------|
| 1 | **TODO-163** | P0 Security: JWT exp, NetworkModule auth, CORS | 2-3 дня | **Blocker** |
| 2 | **TODO-136** | Rate Limits + per-tenant quotas | 1-2 нед | Planned (wave 6f³) |
| 3 | **TODO-033a** | LRU Evictor (slice of 033) | 3-5 дней | Planned (wave 6c) |
| 4 | **TODO-141** | Docker configs | 3-5 дней | Planned (wave 6f⁴) |
| 5 | **TODO-164** | P2 Security: RS256, HSTS, cluster TLS | 1-2 нед | Planned (wave 6f⁴) |

**Не блокеры (отложены):**
- TODO-041 (Multi-Tenancy) → v3.0, namespace isolation достаточна
- TODO-043 (S3 Bottomless) → v3.0, PostgreSQL fallback достаточен
- TODO-040 (Tiered Storage) → v3.0, LRU evictor достаточен

### Namespace Isolation (мини-multi-tenancy для v2.0)

Не полный TODO-041, а минимальная изоляция поверх TODO-136:

```
Что реализовать:
1. Tenant ID из JWT sub claim → prefix для всех map names
2. Per-connection config: max_entry_count, max_connections, ops/sec limit
3. PostgreSQL: tenant_id column в existing tables (NOT отдельные schemas)
4. Admin API: CRUD tenant configs (limits, status, plan)

Что НЕ нужно:
- Per-tenant partitioning (TODO-041)
- Tenant-aware cluster routing (TODO-041)
- Billing integration (TODO-151 covers this)
- Per-tenant PostgreSQL schemas
```

**Effort:** 3-5 дней поверх TODO-136. Включить как subtask в TODO-152.

---

## 5. Операционные процессы (для соло-основателя)

### Deployment

```bash
# Single-command deploy
docker compose -f docker-compose.cloud.yml up -d

# Update
docker compose pull && docker compose up -d --no-deps topgun-server

# Rollback
docker compose up -d --no-deps topgun-server:<previous-tag>
```

### Мониторинг (минимальный viable)

| Что | Инструмент | Стоимость |
|-----|-----------|----------|
| Uptime | UptimeRobot (free, 50 monitors) | $0 |
| Metrics | Grafana Cloud free tier | $0 |
| Logs | Docker logs + Loki (self-hosted) | $0 |
| Alerts | UptimeRobot + Grafana alerts → Telegram | $0 |

**Total monitoring cost: $0** (до 50 клиентов)

### Backup

- PostgreSQL: pg_dump cron → Cloudflare R2 (ежедневно)
- TopGun state: recoverable from PostgreSQL (source of truth for cold data)
- In-memory hot data: не бэкапится (CRDT replicas на клиентах = implicit backup)

---

## 6. Риски и открытые вопросы

### Решённые вопросы (через итерации)

| Вопрос | Решение |
|--------|---------|
| RAM bottleneck? | LRU evictor + PostgreSQL fallback. 28GB вмещает 54M записей |
| Multi-tenancy для cloud? | Namespace isolation (3-5 дней), не full TODO-041 (4-6 нед) |
| Pricing vs конкуренты? | $25 Pro (= Supabase, 2x connections) |
| Free tier economics? | $0.25/tenant, 50 free на CCX33 = $12.50/мес |
| Migration free→paid? | Нет миграции (shared instance), upgrade = изменение лимитов |

### Открытые вопросы (требуют проработки)

| Вопрос | Влияние | Когда решать |
|--------|---------|-------------|
| Self-service signup flow (UI/UX) | Как клиент создаёт аккаунт, получает endpoint, API key? | При реализации TODO-152 |
| Tenant provisioning automation | Скрипт vs API для создания tenant config | При реализации TODO-152 |
| Abuse prevention на free tier | Что если кто-то создаёт 100 free accounts? | При реализации TODO-136 |
| GDPR compliance | Data deletion requests, DPA | Перед EU-клиентами |
| SLA определение для Enterprise | Uptime commitment, response times | При первом Enterprise клиенте |
| Billing proration | Upgrade/downgrade mid-cycle | При интеграции Paddle |

---

## 7. Зрелость плана

**Текущая оценка: 6/10**

| Аспект | Зрелость | Что сделано | Что осталось |
|--------|----------|------------|-------------|
| Ценообразование | 7/10 | Сравнение с 10 конкурентами, psychological pricing validated | A/B тестирование, feedback от beta users |
| Архитектура | 6/10 | RAM analysis, namespace isolation design, security audit | Реализация, нагрузочное тестирование multi-tenant |
| Юнит-экономика | 7/10 | Per-tenant costs, break-even, margin projections | Реальные данные после запуска |
| Операции | 4/10 | Мониторинг-стек выбран, backup plan | Runbooks, incident response, on-call |
| Security | 5/10 | Аудит проведён, P0 найдены | P0 fixes, penetration testing |
| Legal | 3/10 | Юрисдикция выбрана | Регистрация, ToS, Privacy Policy, DPA |
| Go-to-market | 5/10 | Каналы определены, content strategy | Execution, community building |

**Что повысит зрелость до 8/10:**
1. Исправить P0 security (TODO-163)
2. Реализовать namespace isolation + rate limits
3. Запустить beta с 5-10 пользователями
4. Собрать реальный feedback по pricing
5. Написать ToS и Privacy Policy
