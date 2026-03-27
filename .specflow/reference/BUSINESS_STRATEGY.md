# TopGun — Бизнес-стратегия и дорожная карта монетизации

> **Дата:** 2026-03-27 (обновлено)
> **Контекст:** Соло-основатель, v1.0 выпущен, v2.0 в разработке (Firebase Killer phase)
> **Ограничение:** Non-native English speaker (письменный через LLM = нативный; разговорный — в развитии)
> **Цель:** Дорожная карта от open-source проекта до прибыльного бизнеса с потенциалом exit
> **Ключевое решение (2026-03-26):** ~~Feature-first стратегия~~ → **Firebase Killer (compressed):** UX-first, Phase 0 validation, enterprise features deferred. SQL + search + offline + scale уже работают — разрыв в UX и онбординге. CEO plan: `~/.gstack/projects/TopGunBuild-topgun/ceo-plans/2026-03-26-firebase-killer-compressed.md`

---

## 1. Текущее положение и конкурентная позиция

### Что уже есть
- v1.0 выпущен: 540+ Rust-тестов, 55 интеграционных, clippy-clean
- Производительность: 560K+ ops/sec throughput (fire-and-forget), 37K confirmed writes/sec (fire-and-wait, sub-2ms median latency). Benchmark 2026-03-27, Apple M1 Max, 200 connections, in-process.
- v2.0 в активной разработке (Firebase Killer phase): Schema System ✓, DataFusion SQL ✓, Tantivy FTS ✓, Query unification ✓, Indexing (Hash/Navigable/Inverted) ✓, P0 Security ✓, RS256 auth ✓
- Apache 2.0 лицензия (уже сменена с BSL)
- Admin Dashboard, React SDK, TypeScript client
- **Текущий фокус:** Phase 0 (validation) → Phase 1 (production core) → soft launch → Phase 2-4 (UX → cloud → Show HN). Enterprise features (DAG, WASM, Connectors, Distributed Locks) отложены до подтверждения спроса.

### Уникальная позиция на рынке

TopGun — **единственный продукт в верхнем правом квадранте**: сильная offline-поддержка + мощные серверные вычисления. Ни один конкурент не закрывает оба направления.

| Конкурент | Offline | Server Compute | SQL / FTS | Модель монетизации | Статус |
|-----------|---------|---------------|-----------|-------------------|--------|
| **Hazelcast** | Нет | Сильный | SQL (limited) | Open-core, $30-50M ARR | Куплен Broadcom (2025) |
| **Ditto** | Сильный (CRDT) | Нет | Нет SQL | Проприетарный SDK, $100-500K контракты | $45M funding |
| **PowerSync** | Да (SQLite) | Нет | SQLite (клиент) | Cloud + self-hosted | $4.5M seed, pre-revenue |
| **ElectricSQL** | Только read-path | Нет | Нет | OSS, pivot в "multi-agent AI" (2026) | $6.1M seed, pre-revenue |
| **Supabase** | Нет | Средний | PostgreSQL | Cloud, $25/мес+ | ~$20-30M ARR |
| **Instant** | Да | Нет | Нет SQL, нет FTS | Cloud | YC-backed |
| **Triplit** | Да | Нет | Свой QL | OSS + cloud | Растущий |
| **Convex** | Нет | Средний | Свой QL | Cloud BaaS | $46M funding, early revenue |
| **RxDB** | Сильный | Нет | Нет SQL | Premium-плагины, ~$150-300/год/dev | Bootstrapped, соло |

**Ключевой вывод:** Ниша local-first/offline-first **недомонетизирована**. RxDB — bootstrapped, ElectricSQL и PowerSync — pre-revenue. Ditto доказывает, что enterprise offline-first может стоить дорого ($45M raised).

---

## 2. Модель монетизации: три потока дохода

### Поток 1: Managed Cloud (TopGun Cloud) — основной

**Что:** Полностью управляемый сервис — клиент создаёт аккаунт, получает endpoint, подключает SDK.
**Детали:** [CLOUD_ARCHITECTURE.md](CLOUD_ARCHITECTURE.md)

**Ценообразование (v2, после сравнения с 10 конкурентами):**

| Тариф | Цена | Connections | Storage | Проекты | Целевой клиент |
|-------|------|------------|---------|---------|---------------|
| **Free** | $0 | 100 concurrent | 500MB | 1 | Оценка, хобби |
| **Pro** | $25/мес | 1,000 concurrent | 5GB | 5 | Инди/стартапы |
| **Team** | $79/мес | 5,000 concurrent | 25GB | 20 | Растущие команды |
| **Enterprise** | $299+/мес | Unlimited | Unlimited | Unlimited | SLA, dedicated, VPC |

**Позиционирование:** За $25 (= цена Supabase Pro) — 2x connections (1000 vs 500) + offline-first + CRDT conflict resolution.

**Модель хостинга:** Shared instance с namespace isolation (все пользователи на одном кластере). Dedicated только для Enterprise по запросу. Полная multi-tenancy (TODO-041) отложена до v3.0 — namespace isolation достаточна для <100 клиентов.

**Юнит-экономика (shared instance на Hetzner CCX33, $60/мес):**

| Сценарий | Revenue | Infra + Paddle | Margin |
|----------|---------|----------------|--------|
| 50 Free + 3 Pro (month 1-3) | $75/мес | $89 | **-15%** (pre-profit) |
| 50 Free + 10 Pro + 2 Team (month 4-6) | $408/мес | $105 | **74%** |
| 30 Free + 20 Pro + 5 Team (month 7-12) | $895/мес | $130 | **85%** |

Break-even: **4 Pro клиента.** Первые 3 месяца убыточны (~$85/мес из кармана). Free tier — маркетинговый расход.

**RAM:** Per-record overhead ~512B. CCX33 (28GB usable) вмещает 54M todo-записей. 60 tenants используют ~27% RAM. LRU evictor (TODO-033a) вытесняет холодные записи в PostgreSQL.

### Поток 2: Enterprise Self-Hosted License (v3.0+)

**Что:** Enterprise-фичи (multi-tenancy, tiered storage, vector search) под BSL лицензией.

**Ценообразование:**
- Self-hosted Enterprise: $15,000-40,000/год per cluster
- По модели Hazelcast (per-node) или Confluent (per-partition)
- Включает: SSO, audit log, multi-tenancy, tiered storage, priority support

### Поток 3: Premium Support & Consulting

**Что:** Для ранних стадий — consulting по внедрению. Позже — SLA-контракты.

| Услуга | Цена |
|--------|------|
| Architecture review (1-time) | $2,000-5,000 |
| Integration consulting (hourly) | $200-300/час |
| Priority support SLA | Включено в Enterprise |

---

## 3. Когда начать зарабатывать: фазовый план

### Стратегическое решение: Firebase Killer — Compressed (2026-03-26)

**~~Feature-first~~ → UX-first.** 5+ VC-конкурентов выпускают обновления ежемесячно. 9+ месяцев без обратной связи от пользователей — неприемлемый риск. SQL + search + offline + scale уже работают. Разрыв — в UX и онбординге, а не в фичах.

**Почему:**
- Ключевые дифференциаторы (CRDT + SQL + FTS + кластеризация) уже реализованы
- Enterprise-фичи (DAG, WASM, Connectors, Distributed Locks) не нужны первым пользователям
- Быстрее к Show HN = быстрее feedback = быстрее к product-market fit
- Позиционирование: "Firestore alternative" (не "Firebase Killer" — Firebase включает auth, hosting, functions)

### Фаза 0: Firebase Killer — 5 этапов (текущая позиция)

**Phase 0: Validation (3-4 недели)**
- CLI Audit & Fix (15 команд → Rust server)
- Getting Started guide + Docker demo data + 3 onboarding paths (Docker, CLI, SDK)

**Phase 1: Production Core (7-9 недель)**
- Simplified RBAC (role→map boolean), Write Concern APPLIED, Index Admin API
- SQL tab в админ-панели, Client SDK `sql()` method, SetupWizard backend
- Lightweight metrics, Docs audit
- → **SOFT LAUNCH:** r/rust, r/selfhosted, GUN.js Discord

**Phase 2: User Experience (6-8 недель)**
- Template apps (todo, chat, e-commerce), "Migrating from Firebase" guide
- "Planned Feature" cleanup, Admin Dashboard polish

**Phase 3: Cloud Readiness (4-5 недель)**
- Rate Limiting, LRU Evictor, Prometheus/OTel, Security Hardening
- Docker prod configs, Backup/Restore, Webhooks, Namespace isolation

**Phase 4: Show HN (2-3 недели)**
- README rewrite, Community setup, Landing page, Demo improvements
- **Show HN: "TopGun — Open-source Firestore alternative with offline-first CRDTs (Rust)"**
- **Доход: $0 (pre-cloud)**

**Итого: 20-27 недель (5-7 месяцев)**

### Фаза 1 (бизнес): Cloud Launch (после Phase 3-4)

**Триггер:** v2.0 Phase 3 complete (cloud readiness) + Show HN done

**Действия:**
1. Зарегистрировать компанию (см. раздел 5)
2. Настроить Paddle для приёма платежей
3. Развернуть TopGun Cloud на Hetzner CCX33 (shared instance, €55/мес)
4. Открыть бету: Free tier для всех, Pro за $25 для ранних adopters
5. Discord-сервер для сообщества

**Цель:** 50+ бесплатных пользователей, 3-5 платящих
**Доход: $75-125/мес (pre-profit, break-even при 4 Pro)**

### Фаза 2: Self-Serve Growth (через 3-6 месяцев после бета-запуска)

**Действия:**
1. Стабилизация cloud платформы
2. Self-service signup (без manual onboarding)
3. Добавить Team-тариф ($79)
4. Контент-маркетинг: сравнения, benchmark-посты
5. Записанные видео-туториалы (вместо conference talks — языковое ограничение)

**Цель:** 10-20 платящих (Pro + Team)
**Доход: $400-1,500/мес, margin 74-85%**

### Фаза 3: Enterprise (через 12-18 месяцев после запуска)

**Триггер:** $2k+ MRR стабильно, есть запросы от enterprise

**Действия:**
1. Начать v3.0: TODO-041 (multi-tenancy) → Cloud Phase B (shared infra)
2. Enterprise тариф $299+/мес
3. Self-hosted Enterprise license ($15-40K/год)
4. Первые enterprise контракты через inbound + async sales (email + Loom)

**Цель:** $5-10k MRR, 2-3 enterprise клиентов
**Доход: $5,000-10,000/мес**

### Фаза 4: Масштабирование или Exit (через 24-36 месяцев)

**Развилка:**

| Путь | ARR | Условие | Результат |
|------|-----|---------|----------|
| **Lifestyle business** | $200-500K | Соло/маленькая команда, прибыльный | $15-40K/мес чистыми |
| **Seed round** | $500K-1M | Нужен рост, готов взять партнёров | $2-5M раунд, нанять 3-5 человек |
| **Acquisition** | $1-3M | Стратегический интерес (Cloudflare, Vercel, Supabase) | $10-30M (8-15x ARR) |

---

## 4. Потенциал exit и мультипликаторы

### Целевые покупатели для TopGun

| Покупатель | Зачем им TopGun | Вероятность |
|-----------|----------------|------------|
| **Cloudflare** | Дополняет Durable Objects + D1 offline-first | Высокая |
| **Supabase** | Real-time layer + offline-first для Postgres | Высокая |
| **Vercel** | Backend data platform для Next.js apps | Средняя |
| **Fly.io** | Edge compute + distributed data | Средняя |
| **Broadcom/Hazelcast** | Расширение IMDG на offline-first | Средняя |

### Мультипликаторы при продаже (2024-2025 рынок)

| ARR | Типичный мультипликатор | Оценка |
|-----|------------------------|--------|
| $1M | 10-15x | $10-15M |
| $3M | 10-15x | $30-45M |
| $10M | 8-12x | $80-120M |
| $30M+ | 8-15x | $240M+ |

**Стратегическая премия** (если покупатель получает уникальную технологию): +2-3x сверху.

**Референсы:**
- GitHub → Microsoft: 25x ARR ($7.5B при ~$300M ARR)
- HashiCorp → IBM: 11x ARR ($6.4B при ~$600M ARR)
- Hazelcast → Broadcom: оценочно 8-15x ARR
- PlanetScale: закрылся несмотря на $105M funding (предупреждение)

---

## 5. Регистрация компании и юрисдикция

### Рекомендация: Stripe Atlas (Delaware LLC)

**Почему:**
- Максимальный кредит доверия у US-клиентов и инвесторов
- $500 за всё: LLC + EIN + Mercury bank + Stripe + registered agent
- 0% корпоративного налога на уровне LLC (pass-through)
- Налог платится только в стране резидентства (на дивиденды)

**Альтернатива (если фокус на EU):** Estonia e-Residency OÜ через Xolo (~€1,500/год)
- 0% на нераспределённую прибыль
- EU VAT reverse charge для B2B
- Проще для EU-клиентов

### Когда регистрировать

**За 2-4 недели до первого платежа.** Не раньше — лишние расходы. Не позже — нельзя принимать платежи на личный счёт.

**Практический триггер:** Когда TopGun Cloud готов к бета-запуску.

### Оценка годовых расходов (первый год)

| Статья | Стоимость |
|--------|----------|
| Stripe Atlas (Delaware LLC) | $500 (разово) |
| Registered agent (год 2+) | $300/год |
| Delaware franchise tax | $300/год |
| Paddle (payment processing) | 5% от выручки |
| Infrastructure (Hetzner) | $360-720/год |
| Domain + DNS | $10/год |
| **Итого (без учёта выручки)** | **~$1,500-2,000/год** |

---

## 6. Приём платежей

### Рекомендация: Paddle → Stripe (по мере роста)

**Фаза 1 (0 → $10k MRR): Paddle**
- Merchant of Record — они юридический продавец
- Обрабатывают VAT, sales tax, инвойсы за вас
- 5% + $0.50 за транзакцию (vs Stripe 2.9% + $0.30)
- Экономия: 10+ часов/мес на налоговом compliance
- Настройка: 1-2 недели на одобрение

**Фаза 2 ($10k+ MRR): Stripe**
- Переход когда enterprise-клиенты требуют custom invoices, PO, net-30
- Stripe Billing + Tax + Invoicing
- К этому моменту нужен бухгалтер в любом случае

---

## 7. Инфраструктурные затраты

### Этапы масштабирования

| Клиентов | Инфраструктура | Стоимость/мес |
|----------|---------------|-------------|
| 0-10 | 1x Hetzner CCX13, Neon free PG, Cloudflare R2 | $15-30 |
| 10-50 | 2x Hetzner (HA), Supabase/self-managed PG | $60-100 |
| 50-200 | K8s на Hetzner (3 nodes), managed PG | $200-500 |
| 200+ | Multi-region, AWS/GCP для глобальности | $1,000+ |

### Break-Even (пересчитано 2026-03-22)

Base cost: $85/мес (Hetzner CCX33 $60 + managed PostgreSQL $25). Paddle: 5%.

| Тариф | Break-even | Клиентов до $1K MRR | Клиентов до $5K MRR |
|-------|-----------|---------------------|---------------------|
| Pro ($25) | 4 клиента | 40 | 200 |
| Team ($79) | 2 клиента | 13 | 64 |
| Enterprise ($299) | 1 клиент | 4 | 17 |

**Реалистичный сценарий (mix: 60% Pro / 30% Team / 10% Enterprise):**
- 30 paying клиентов → ~$1,567 MRR
- Infrastructure: $85/мес + Paddle $78 = $163
- **Чистыми: ~$1,400/мес**
- 100 paying клиентов → ~$5,220 MRR → **чистыми ~$4,960/мес**

**Честная оценка:** При новых ценах ($25 Pro вместо $99 Starter) путь к $5K MRR длиннее — нужно ~100 paying клиентов вместо 52. Это trade-off: больше adoption (ниже барьер входа) vs медленнее revenue growth.

---

## 8. Маркетинг и популяризация

### Языковое ограничение и стратегия коммуникации

**Факт:** Основатель — non-native English speaker (нет свободного разговорного).

**Почему это не блокер:**
- Developer tools на 95% продаются через текст (docs, README, blog, GitHub, Discord)
- Письменный английский через LLM = неотличим от нативного
- Успешные non-native founders в dev tools: Nikita Shamgunov (Neon, RU), Glauber Costa (Turso, BR), Talip Ozturk (Hazelcast, TR), Guillermo Rauch (Vercel, AR)
- Код, бенчмарки и API design говорят за себя — акцент основателя не влияет на adoption

**Рабочий процесс для контента:**
1. Черновик на русском (или broken English) → LLM-редактура до native level → публикация
2. Discord/GitHub ответы: суть ответа → LLM формулирует грамотно
3. Шаблоны + FAQ для типовых ответов — снижают нагрузку на живое общение
4. Только текстовые каналы на ранних стадиях (никаких voice/video calls)

**Что отложить:**
- Conference talks → заменить записанными видео (можно перезаписывать до идеала)
- Live enterprise sales calls → async через email + Loom-видео до $10K MRR
- При $10K MRR: нанять part-time customer success ($20-30/час) для live calls

**Параллельная инвестиция:**
- Italki/Preply разговорная практика, $10-15/час, 2-3 раза/неделю
- Через 6-12 месяцев уровень будет достаточным для calls
- Не блокер для старта — параллельный процесс

### Каналы (по ROI для соло-основателя, адаптированы под языковое ограничение)

| Канал | Усилия | Язык. барьер | Срок до результата | Ожидаемый эффект |
|-------|--------|-------------|-------------------|-----------------|
| **Show HN** | Низкие (1 пост) | Нулевой (текст через LLM) | 1-3 дня | 5-50K визитов, 50-500 stars, 0-10 signups |
| **Технический блог** | Высокие (4-8ч/статья) | Нулевой (LLM-редактура) | 3-6 мес (SEO) | Лучший долгосрочный ROI; 60% трафика dev tools |
| **Twitter/X** | Средние (ежедневно) | Низкий (короткие посты + скриншоты) | 2-4 мес | Build-in-public; 1-10K followers за 6 мес |
| **Reddit** (r/rust, r/programming) | Низкие | Низкий (текст) | Сразу | Валидация; осторожно с self-promo |
| **Discord** | Средние (постоянно) | Низкий (текст + шаблоны) | 3-6 мес | Retention, power users, обратная связь |
| **Telegram (RU)** | Низкие | Нулевой (родной язык) | 1-3 мес | Первые пользователи, быстрый feedback |
| ~~Конференции~~ | ~~Высокие~~ | ~~Высокий~~ | — | Заменить на записанные видео + YouTube |
| **YouTube (записи)** | Средние | Средний (можно перезаписать) | 3-6 мес | Растущий канал для dev tools |

### Русскоязычный рынок как стартовая площадка

**Зачем:** Первые 5-10 пользователей/клиентов проще найти на родном языке. Это снижает барьер для support и обратной связи.

**Каналы:**
- Telegram-канал на русском (параллельно с Discord на английском)
- Хабр — технические статьи (аналог HN для RU)
- Russian-speaking dev communities (Telegram-группы по Rust, TypeScript)

**Ограничение:** Русскоязычный рынок значительно меньше и менее платёжеспособен. Использовать для валидации и первых отзывов, не как основной рынок.

### Контент-стратегия: что работает

1. **"How we built X"** — Rust benchmarks, архитектурные решения. Разработчики любят глубину.
2. **Сравнения** — "TopGun vs Firebase vs Supabase Realtime" (SEO-золото, purchase-intent трафик)
3. **Problem-solution** — "Why CRDTs solve real-time collaboration better than OT"
4. **Бенчмарки** — Latency, throughput. Разработчики шарят такое активно.

**Каденция:** 2 поста/месяц. Качество > количество.
**Процесс:** Черновик (RU/EN) → LLM → вычитка → публикация. Среднее время: 4-6 часов на пост.

### Подготовка Show HN (ключевой момент)

**Заголовок:** `Show HN: TopGun – Open-source real-time data platform with offline-first CRDTs (Rust)`

**Что нужно к моменту Show HN:**
- [ ] Рабочая интерактивная демо (sync showcase)
- [ ] Чистый README с clear value prop
- [ ] Quick start: `npm install @topgunbuild/client` → рабочий пример за 5 минут
- [ ] Discord-сервер
- [ ] Документация на topgun.build
- [ ] 1-2 опубликованных блог-поста для контекста
- [ ] Подготовить ответы на типовые HN-вопросы заранее (через LLM)

---

## 9. Продукт vs платформа: стратегический выбор

### Рекомендация: **И то, и другое (staged)**

| Стадия | Фокус | Почему |
|--------|-------|--------|
| **Сейчас → v2.0** | Самостоятельный продукт (SDK + Cloud) | Нужна adoption база; SDK — это growth engine |
| **$5k+ MRR** | + Платформа для решений (embeddable) | Появятся клиенты, строящие НА TopGun |
| **$20k+ MRR** | + Marketplace (extensions, connectors) | Экосистема как moat |

**Аналогия:** Supabase начал как "Firebase alternative" (продукт), затем стал платформой (auth + storage + edge functions + realtime). TopGun может пройти тот же путь.

---

## 10. Портрет идеального клиента (ICP)

### Сегмент 1: Стартапы с collaborative features (Tier 1 adoption)

- **Кто:** 2-10 инженеров, строят SaaS с real-time элементами
- **Проблема:** Нужен real-time sync для одной фичи, не хотят менять весь стек
- **Примеры:** Kanban-доски, project management, design tools
- **ARPPU:** $99-299/мес
- **Как найти:** Show HN, Twitter, "real-time collaboration" поисковые запросы
- **Решающий фактор:** "Работает с моим существующим Postgres"

### Сегмент 2: Offline-critical приложения (Tier 2-3)

- **Кто:** Field service, logistics, healthcare, retail POS
- **Проблема:** Приложение ДОЛЖНО работать без интернета
- **Примеры:** Мобильные CRM для полевых агентов, инвентаризация на складе
- **ARPPU:** $299-999/мес
- **Как найти:** ~~Конференции~~, enterprise inbound, партнёрства с системными интеграторами
- **Решающий фактор:** "Работает offline с автоматическим слиянием конфликтов"
- **Языковая адаптация:** Async sales (email + Loom). При $10K MRR — нанять sales contractor для calls

### Сегмент 3: Real-time analytics / IoT (v2.0+)

- **Кто:** Компании с потоковой обработкой данных
- **Проблема:** Нужен stream processing + real-time dashboards
- **Примеры:** IoT мониторинг, trading dashboards, live analytics
- **ARPPU:** $999+/мес
- **Как найти:** Enterprise sales, Hazelcast/Kafka replacement stories
- **Решающий фактор:** "SQL + streaming + real-time push в одной платформе"

---

## 11. Конкретный план действий

### Сейчас: Firebase Killer — Phase 0-1 (validation + production core)

| # | Действие | Усилия | Важность |
|---|----------|--------|----------|
| 1 | Phase 0: CLI Audit, Getting Started guide, Docker demo data, 3 onboarding paths | 3-4 нед | **Критическая** |
| 2 | Phase 1: RBAC, Write Concern, Index Admin API, SQL tab + SDK method, SetupWizard | 7-9 нед | **Критическая** |
| 3 | Записаться на Italki/Preply (разговорный EN, 2-3 раза/неделю) | 1 час/setup | Средняя |

### После soft launch: UX + маркетинговая подготовка

| # | Действие | Усилия | Важность |
|---|----------|--------|----------|
| 4 | Phase 2: Template apps, Firebase migration guide, Planned Feature cleanup | 6-8 нед | Высокая |
| 5 | Открыть Discord-сервер (text-only, без voice каналов) | 1 час | Высокая |
| 6 | Написать первый блог-пост: "Why I'm building a real-time data platform in Rust" | 4-8 часов | Высокая |
| 7 | Начать Twitter/X build-in-public | 30 мин/день | Средняя |

### Cloud readiness + Show HN

| # | Действие | Усилия | Важность |
|---|----------|--------|----------|
| 8 | Phase 3: Rate Limits, LRU Evictor, Metrics, Security, Docker, Webhooks | 4-5 нед | Критическая |
| 9 | Phase 4: README rewrite, Community, Landing page, Demo, Show HN | 2-3 нед | Критическая |
| 10 | Подготовить Stripe Atlas заявку + зарегистрировать компанию | $500 + 1 час | Критическая |
| 11 | Настроить Paddle (занимает 1-2 недели) | 1 час | Критическая |
| 12 | Развернуть TopGun Cloud на Hetzner CCX33 | 1-2 дня | Критическая |
| 13 | Написать 2-3 comparison поста (SEO) | 1 неделя | Высокая |

---

## 12. Риски и митигация

| Риск | Вероятность | Митигация |
|------|------------|-----------|
| AWS/Cloudflare клонирует | Средняя | Скорость инновации + community moat + CRDT expertise |
| PlanetScale-сценарий (unit economics) | Низкая | Hetzner (3-5x дешевле AWS), break-even при 4 Pro клиентах |
| Нет adoption | Средняя | Tier 1 adoption path (не требует замены стека) |
| Burnout (соло) | Высокая | Ставить реалистичные сроки; нанять первого помощника при $5k MRR |
| Enterprise sales cycle слишком длинный | Средняя | Фокус на self-serve (Starter/Pro) до $10k MRR |
| Языковой барьер на enterprise calls | Средняя | Async-first sales (email + Loom); contractor при $10K MRR; параллельное изучение EN |
| Community moderation на EN | Низкая | Шаблоны ответов + LLM; text-only Discord; FAQ-бот |

---

## 13. Ключевые метрики для отслеживания

| Метрика | Цель (6 мес) | Цель (12 мес) | Цель (24 мес) |
|---------|-------------|--------------|--------------|
| GitHub stars | 1,000 | 5,000 | 15,000 |
| npm weekly downloads | 500 | 5,000 | 20,000 |
| Discord members | 100 | 500 | 2,000 |
| Free tier users | 50 | 200 | 1,000 |
| Paying customers | 3-5 | 20-30 | 80-100 |
| MRR | $75-125 | $800-1,500 | $5,000-10,000 |
| Blog monthly visitors | 2,000 | 10,000 | 50,000 |

*Метрики пересчитаны (2026-03-22) с учётом сниженных цен ($25 Pro vs $99 Starter). Путь к $5K MRR длиннее, но adoption выше.*

---

## Итого: Timeline to Exit

```
2026 Q2:    Phase 0 (Validation) + Phase 1 start (Production Core)
2026 Q2-Q3: Phase 1 complete → SOFT LAUNCH (r/rust, r/selfhosted)
2026 Q3:    Phase 2 (UX: templates, docs, Firebase guide)
2026 Q3-Q4: Phase 3 (Cloud Readiness: Rate Limits, Security, Docker)
2026 Q4:    Phase 4 (Show HN) → Cloud launch → 50+ free, 3-5 paid → $75-125 MRR
2027 Q1-Q2: Self-serve рост → 20-30 paid → $800-1,500 MRR
2027 Q3-2028: v3.0 начало (Multi-Tenancy, S3, Enterprise), Team/Enterprise → $3-5K MRR
2028-2029:  Развилка — lifestyle ($5-15K/мес) или seed round ($1-3M)
2029-2030:  При seed: $500K-1M ARR → acquisition target ($5-15M+)
```

**Самый короткий путь к exit:** 3-4 года при агрессивном росте.
**Самый вероятный путь:** Прибыльный lifestyle business через 18-24 месяца.
**Оба варианта хорошие** — решение принимать при достижении $5K MRR.

**Преимущество Firebase Killer подхода:** Быстрее к Show HN (5-7 мес вместо 12+). Ключевые дифференциаторы (CRDT + SQL + FTS + кластеризация) уже работают. Enterprise-фичи (DAG, WASM, Connectors) будут добавлены по запросу пользователей.

**Честная оценка зрелости плана: 7/10** (выросла с 6/10 после: CEO review + eng review + product concept finalization + benchmark verification). P0 security исправлен (SPEC-137). Indexing завершён (SPEC-155). Основные оставшиеся риски: непроверенный pricing (нужен feedback от beta users), отсутствие legal framework (ToS, Privacy Policy).
