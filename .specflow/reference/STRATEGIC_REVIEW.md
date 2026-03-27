# TopGun Strategic Review

> **Назначение:** Живой документ для стратегического контроля. Проверяется Claude в начале каждой бизнес-сессии.
> **Обновляется:** После каждого стратегического решения или выявления нового риска.
> **Связан с:** [BUSINESS_STRATEGY.md](BUSINESS_STRATEGY.md), [CLOUD_ARCHITECTURE.md](CLOUD_ARCHITECTURE.md)

---

## 1. Decision Log (журнал решений)

*Каждое стратегическое решение фиксируется с обоснованием и отклонёнными альтернативами. Позволяет пересмотреть решения при изменении контекста.*

| Дата | Решение | Обоснование | Отклонённые альтернативы | Условие для пересмотра |
|------|---------|-------------|-------------------------|----------------------|
| 2026-03-22 | Cloud: shared instance, не single-tenant | 50 free users = $700/мес при single-tenant; shared = $0.25/user | Single-tenant (каждый в Docker), full multi-tenancy (слишком сложно для v2.0) | Если namespace isolation создаёт security-инциденты или не масштабируется >100 tenants |
| 2026-03-22 | Pricing: $25 Pro (не $99 Starter) | Рыночная норма $25 (Supabase, Convex). $99 — 4x дороже рынка | $99 Starter (выше margin, но ниже adoption), freemium only, pay-per-use | Если conversion rate free→pro <2% — пересмотреть value prop или pricing |
| 2026-03-22 | Cloud launch без multi-tenancy (TODO-041) | Namespace isolation достаточна для <100 tenants. TODO-041 = 4-6 недель | Отложить cloud до v3.0 (дольше до revenue) | Если достигнуто 100 tenants — начать TODO-041 |
| 2026-03-22 | Dual-phase cloud: Phase A (v2.0 shared) → Phase B (v3.0 multi-tenant) | Быстрее выход на рынок, валидация спроса до инвестиций в multi-tenancy | Сразу multi-tenant (медленнее, но масштабируется) | При ~100 клиентах или при первом enterprise-запросе на isolation |
| 2026-03-22 | Первые 3 месяца убыточны (~$85/мес) | Нормальная инвестиция; free tier = маркетинговый расход | Без free tier (быстрее break-even, но хуже adoption) | Если убытки >$200/мес или нет роста через 6 месяцев |
| 2026-03-22 | Cloud portal auth: Clerk (не BetterAuth) | $0 для 50K MAU, pre-built UI, zero maintenance. Уже работает в `examples/notes-app` — проверенная интеграция. Clerk = только portal login, не data plane. TopGun data connections используют собственные HS256 JWT | BetterAuth self-hosted (больше контроля, но 1-2 нед UI + hosting + maintenance) | Если Clerk меняет pricing >$50/мес или sunset — миграция на BetterAuth |
| 2026-03-22 | TODO-163 (P0 security) — сразу после SPEC-136 | demo.topgun.build уже публичен с уязвимостями. 2-3 дня. Смежный код с SPEC-136 | Отложить до wave 6f² (позже, но рискованнее) | — |
| 2026-03-20 | Apache 2.0 core + BSL enterprise | Open-core стандарт для dev tools. Apache для adoption, BSL для enterprise revenue | Full Apache (нет enterprise moat), proprietary (нет adoption) | Если конкурент fork'ает и обгоняет — рассмотреть SSPL или Elastic License |
| 2026-03-25 | ~~Feature-first: полное open-source ядро до cloud launch~~ **SUPERSEDED 2026-03-26** | Show HN с полным набором дифференциаторов (DAG, WASM, Connectors, Indexing, Locks, RBAC) — мощнее, чем ранний запуск с минимальным feature set. Один шанс на first impression | Ранний cloud launch после SQL+Shapes (быстрее к revenue, но слабый launch) | — (заменено решением от 2026-03-26) |
| 2026-03-26 | **Firebase Killer — Compressed:** UX-first, отложить enterprise-фичи | 5+ VC-конкурентов выпускают обновления ежемесячно. 9+ мес без обратной связи от пользователей — неприемлемый риск. SQL + search + offline + scale уже работают. Разрыв — в UX и онбординге, а не в фичах. /office-hours + /plan-ceo-review (SELECTIVE EXPANSION). Phase 0: Getting Started guide (валидация). Упрощённый RBAC. Индексы через Admin API. Позиционирование: "Firestore alternative". | Feature-first (9+ мес, слишком долго), Speed Run (2-3 нед, слишком мало фич), Full v2.0 design doc (5-7 мес, Distributed Locks + Schema Migrations не нужны первым пользователям) | Если первые 50 пользователей запросят enterprise-фичи (DAG, WASM, Connectors) — пересмотреть. Если Show HN провалится — Direct outreach к GUN.js/Firebase community |

---

## 2. Risk Register (реестр рисков)

*Отслеживает идентифицированные риски. Пересматривается каждую бизнес-сессию.*

| ID | Риск | Вероятность | Импакт | Статус | Митигация | Триггер для эскалации |
|----|------|------------|--------|--------|-----------|----------------------|
| R-001 | P0 security bugs (JWT, auth) позволят атаку на production | Высокая | Критический | **FIXED** (SPEC-137, SPEC-138) | TODO-163 ✓, TODO-169 ✓. Remaining P2 hardening in TODO-164 | — |
| R-002 | Pricing слишком низкий — не хватает на жизнь | Средняя | Высокий | Мониторинг | Пересмотр при $2K MRR. Добавить Team ($79) и Enterprise ($299) | Если после 6 мес <$500 MRR |
| R-003 | Pricing слишком высокий — нет adoption | Средняя | Высокий | Снижен | Снизили с $99 до $25. Мониторить conversion | Если conversion free→pro <1% через 3 мес |
| R-004 | Burnout (соло-основатель, dev + biz + support) | Высокая | Критический | **OPEN** | Реалистичные сроки. Первый найм при $3K MRR (part-time support) | Если работа >60ч/нед >2 мес подряд |
| R-005 | Namespace isolation leak (tenant A видит данные tenant B) | Низкая | Критический | Latent | Тесты изоляции. Code review при реализации | Любой инцидент утечки данных |
| R-006 | Нет product-market fit | Средняя | Критический | Unknown | Show HN feedback, early user interviews, usage analytics | Если 0 organic signups через 2 мес после launch |
| R-007 | Legal exposure (нет ToS, Privacy Policy, DPA) | Средняя | Высокий | **TODO-166 создан** | LLM-генерация + optional legal review ($200-500). Wave 8b | Первый платящий клиент |
| R-008 | AWS/Cloudflare создаёт конкурирующий продукт | Низкая | Высокий | Мониторинг | Скорость инновации + community moat + CRDT expertise | Анонс конкурента |
| R-009 | Hetzner outage убивает all tenants (single point of failure) | Низкая | Критический | Latent | Backup в R2. При $5K MRR — второй регион | Первый значительный outage |
| R-010 | Языковой барьер тормозит enterprise sales | Средняя | Средний | Мониторинг | Async-first (email + Loom). Italki/Preply. Contractor при $10K MRR | Первый потерянный enterprise deal из-за языка |

---

## 3. Open Strategic Questions (открытые вопросы)

*Вопросы, требующие решения. Отсортированы по срочности.*

### Требуют решения ДО launch

- [x] **Self-service signup flow:** ~~Как пользователь создаёт аккаунт?~~ **Решено: Clerk** (50K MAU free). OAuth (GitHub/Google) встроен. Clerk = portal auth only, TopGun data connections = собственные HS256 JWT. Adapter-better-auth остаётся для SDK users
- [ ] **Terms of Service + Privacy Policy:** Шаблон или юрист? Какой шаблон-сервис использовать? (Termly, iubenda, или LLM-генерация + юрист review)
- [ ] **Abuse prevention:** Что если кто-то создаёт 100 free accounts? Rate limit по IP? Email verification? Phone verification?
- [ ] **Monitoring и alerting:** Кто реагирует на outage ночью? Допустимое время простоя?
- [ ] **GDPR:** Нужен ли DPA (Data Processing Agreement)? Где хранить данные EU-клиентов? (Hetzner = EU, ок)

### Требуют решения в первые 3 месяца после launch

- [ ] **Billing proration:** Upgrade/downgrade mid-cycle. Как Paddle это обрабатывает?
- [ ] **Churn reduction:** Какие метрики мониторить? (usage drop, login frequency, connection count trend)
- [ ] **Support SLA:** Время ответа для free vs pro vs team? Какой канал? (Discord vs email vs in-app)
- [ ] **Usage analytics:** Какие метрики собирать? Privacy implications? PostHog self-hosted?
- [ ] **Onboarding flow:** Как new user получает value за первые 5 минут?

### Стратегические (решать при достижении milestone)

- [ ] **При $2K MRR:** Нужен ли первый найм? Кто: support, dev, marketing?
- [ ] **При $5K MRR:** Lifestyle business vs seed round? Критерии решения?
- [ ] **При первом enterprise-запросе:** Готов ли к enterprise sales cycle (3-6 мес)? Legal review?
- [ ] **Python SDK (TODO-142):** Когда? Есть ли спрос? Или лучше REST API?

---

## 4. Blind Spot Checklist (слепые зоны соло-основателя)

*Типичные ошибки и упущения. Проверять ежемесячно.*

### Финансы и Legal
- [ ] **Банковский счёт отдельно от личного** — не смешивать бизнес и личные финансы
- [ ] **Резервный фонд:** 3-6 месяцев расходов на инфраструктуру до первого дохода
- [ ] **Налоговый учёт:** Фиксировать все расходы с первого дня (даже до регистрации)
- [ ] **Intellectual property:** Все контрибуторы должны подписать CLA или работать under Apache 2.0
- [ ] **Liability insurance:** Нужен ли для SaaS? (обычно не нужен до enterprise, но проверить)

### Продукт
- [ ] **Не строить в вакууме:** Получить feedback от 5+ потенциальных пользователей ДО launch
- [ ] **Feature creep:** v2.0 scope уже большой. Резать scope, не добавлять
- [ ] **Monitoring собственного продукта:** Если cloud упал — узнать раньше клиента
- [ ] **Data portability:** Может ли клиент экспортировать свои данные? (GDPR requirement + trust signal)
- [ ] **Backup recovery test:** Не просто делать backup — проверять, что restore работает

### Операции
- [ ] **Bus factor = 1:** Задокументировать всё для recovery (server access, DNS, Paddle, bank, domain registrar)
- [ ] **Incident response:** Что делать при data breach? Кого уведомлять? (GDPR: 72 часа)
- [ ] **Changelog:** Публичный changelog для клиентов. Они хотят знать что меняется
- [ ] **Deprecation policy:** Как предупреждать о breaking changes? (минимум 30 дней)

### Маркетинг и продажи
- [ ] **Не продавать — решать проблему:** Фокус на pain points клиента, не на фичах
- [ ] **Testimonials и case studies:** Просить первых клиентов о feedback (даже free-tier)
- [ ] **Analytics на сайте:** Знать откуда приходят посетители (Plausible — privacy-friendly, EU-hosted)
- [ ] **Email marketing:** Собирать email с waitlist, но не спамить. Ежемесячный newsletter максимум

### Личное
- [ ] **Границы работы:** Установить рабочие часы, не работать 24/7
- [ ] **Celebrate wins:** Отмечать milestones (первый star, первый user, первый payment)
- [ ] **Network:** Общаться с другими indie founders (IndieHackers, Twitter, Telegram-чаты)
- [ ] **Health:** Физическая активность, сон. Burnout убивает больше стартапов чем конкуренты

---

## 5. Session Log (лог сессий)

*Краткая запись каждой бизнес-сессии для непрерывности.*

### 2026-03-20: Initial Strategy
- Создан BUSINESS_STRATEGY.md
- Исследованы 10 конкурентов, мультипликаторы, exit-стратегии
- Определены 3 потока дохода, ICP, каналы маркетинга

### 2026-03-21: Docs & Demo Audit
- Аудит docs-astro (8.5/10) и sync-lab demo (B+ для Show HN)
- Созданы TODO-159 (demo), TODO-160 (README), TODO-161 (social media)
- Добавлены TODO-136-142 (v2.0 gaps для cloud)
- Языковое ограничение: добавлена адаптация стратегии для non-native speaker

### 2026-03-22: Deep Stress-Test
- **RAM analysis:** Per-record 512B overhead, CCX33 вмещает 54M records. RAM не bottleneck
- **Security audit:** Найдены 2x P0 (JWT exp disabled, NetworkModule no auth). Созданы TODO-163-165
- **Pricing revolution:** $99→$25 Pro после сравнения с Supabase/Convex/Neon/Turso
- **Cloud architecture:** Single-tenant → shared instance. Создан CLOUD_ARCHITECTURE.md
- **Free tier economics:** 50 free users = $12.50/мес, не $700
- **Зрелость плана:** оценена в 6/10. Каждый вопрос вскрывает пробелы — это нормально
- **Storage strategy:** TODO-033 split на 033a (LRU evictor) для cloud + full 033 для v3.0
- **Multi-tenancy vs cloud:** Namespace isolation (3-5 дней) вместо full TODO-041 (4-6 недель)

### 2026-03-27: Product Concept Audit + CEO Review v2
- **Product concept finalized** (TopGun-Product-Concept-Final.md) — positioning: "Firestore alternative with offline-first CRDTs"
- **CEO review v2:** 4 concept overreach gaps found and fixed (SQL from SDK, row-level RBAC, S3 tiering, zero infrastructure claim)
- **New section added to concept:** "Что TopGun НЕ заменяет" (auth, hosting, functions, analytics, push notifications)
- **TODO-201 created:** Client SDK `sql()` method — thin wrapper over WS → QueryService → DataFusion. Phase 1, 2-3 days.
- **BUSINESS_STRATEGY.md synced** with Firebase Killer pivot (was stale — still said "Feature-first")
- **Competitive table updated** in BUSINESS_STRATEGY.md — added Instant, Triplit, Convex, SQL/FTS column
- **Benchmark verified (earlier today):** 560K fire-and-forget, 37K fire-and-wait, sub-2ms p50 median latency (M1 Max, 200 connections)
- **TODO.md alignment:** 90% aligned with product concept. No structural changes needed.
- **Plan maturity raised:** 6/10 → 7/10 (CEO + eng review + concept + benchmarks all done)

### 2026-03-22 (continued): Strategic Advisor System + Decisions
- Создана система стратегического контроля: STRATEGIC_REVIEW.md + memory-записи + протокол
- Первый стратегический обзор: выявлены R-001 (P0 security для demo.topgun.build), R-004 (burnout от планирования), R-007 (ToS)
- **Решение: Clerk** для cloud portal auth (50K MAU free, pre-built UI, zero maintenance)
- **Решение: TODO-163** сразу после SPEC-136 (не откладывать)
- **Создан TODO-166** (ToS/PP) — закрывает R-007
- DataFusion SQL (TODO-091) уже реализован, SPEC-136 (Shapes) в процессе
- Наблюдение: 3 дня планирования, 0 кода. Пора возвращаться к реализации

---

## 6. Strategic Advisor Protocol

*Инструкция для Claude при бизнес-обсуждениях.*

### При начале бизнес-сессии:
1. Прочитать этот документ
2. Проверить Open Strategic Questions — есть ли срочные без ответа?
3. Проверить Risk Register — изменился ли статус рисков?
4. Проверить Blind Spot Checklist — что давно не проверялось?
5. Сообщить пользователю о 1-3 самых важных наблюдениях

### При каждом стратегическом решении:
1. Записать в Decision Log с обоснованием и альтернативами
2. Обновить Risk Register если решение создаёт/закрывает риски
3. Задать "а что если?" вопрос — проверить решение на прочность

### Ежемесячно (напомнить пользователю):
1. Пересмотреть Risk Register — актуальны ли митигации?
2. Пройти Blind Spot Checklist
3. Обновить метрики в BUSINESS_STRATEGY.md
4. Записать Session Log
