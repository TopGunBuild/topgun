# TopGun — Бизнес-стратегия и дорожная карта монетизации

> **Дата:** 2026-03-20
> **Контекст:** Соло-основатель, v1.0 выпущен, v2.0 в разработке (wave 6c)
> **Ограничение:** Non-native English speaker (письменный через LLM = нативный; разговорный — в развитии)
> **Цель:** Дорожная карта от open-source проекта до прибыльного бизнеса с потенциалом exit

---

## 1. Текущее положение и конкурентная позиция

### Что уже есть
- v1.0 выпущен: 540+ Rust-тестов, 55 интеграционных, clippy-clean
- Производительность: 200,000 ops/sec (рост с 100 в 2000 раз)
- v2.0 в активной разработке: Schema System готов, DataFusion SQL следующий
- Apache 2.0 лицензия (уже сменена с BSL)
- Admin Dashboard, React SDK, TypeScript client

### Уникальная позиция на рынке

TopGun — **единственный продукт в верхнем правом квадранте**: сильная offline-поддержка + мощные серверные вычисления. Ни один конкурент не закрывает оба направления.

| Конкурент | Offline | Server Compute | Модель монетизации | Статус |
|-----------|---------|---------------|-------------------|--------|
| **Hazelcast** | Нет | Сильный | Open-core, $30-50M ARR | Куплен Broadcom (2025) |
| **Ditto** | Сильный | Нет | Проприетарный SDK, $100-500K контракты | $45M funding |
| **PowerSync** | Средний | Нет | Cloud + self-hosted | $4.5M seed, pre-revenue |
| **ElectricSQL** | Средний | Нет | OSS, cloud в разработке | $6.1M seed, pre-revenue |
| **Supabase** | Нет | Средний | Cloud, $25/мес+ | ~$20-30M ARR |
| **RxDB** | Сильный | Нет | Premium-плагины, ~$150-300/год/dev | Bootstrapped, соло |
| **Convex** | Нет | Средний | Cloud BaaS | $46M funding, early revenue |

**Ключевой вывод:** Ниша local-first/offline-first **недомонетизирована**. RxDB — bootstrapped, ElectricSQL и PowerSync — pre-revenue. Ditto доказывает, что enterprise offline-first может стоить дорого ($45M raised).

---

## 2. Модель монетизации: три потока дохода

### Поток 1: Managed Cloud (TopGun Cloud) — основной

**Что:** Полностью управляемый сервис — клиент создаёт аккаунт, получает endpoint, подключает SDK.

**Ценообразование (usage-based + tiers):**

| Тариф | Цена | Включено | Целевой клиент |
|-------|------|----------|---------------|
| **Free** | $0 | 1 проект, 10 concurrent connections, 100MB storage | Хобби, оценка |
| **Starter** | $99/мес | 5 проектов, 100 connections, 5GB storage, community support | Инди/стартапы |
| **Pro** | $299/мес | 20 проектов, 500 connections, 25GB, email support, SSE, shapes | Растущие команды |
| **Enterprise** | $999+/мес | Unlimited, SLA, dedicated support, VPC, custom domain | Средний/крупный бизнес |

**Юнит-экономика (per-tenant cost):**

| Ресурс | Стоимость |
|--------|----------|
| WebSocket сервер (amortized, 100 conn) | $0.50-1.00/мес |
| PostgreSQL (shared) | $1-3/мес |
| S3/R2 storage (1GB) | $0.02/мес |
| Bandwidth (5GB) | $0.45/мес |
| **Итого per tenant** | **$2-5/мес** |

Gross margin: **95-97%** — один из лучших показателей в SaaS.

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

### Фаза 0: Сейчас → v2.0 готов (текущая позиция)

**Действия:**
- Завершить v2.0 (DataFusion SQL, DAG, Connectors)
- Параллельно: landing page с waitlist для TopGun Cloud
- Параллельно: 2 блог-поста/месяц (технические)
- **Доход: $0**

### Фаза 1: Бета-запуск Cloud (v2.0 + 1-2 месяца)

**Триггер:** v2.0 feature-complete (SQL + Shapes как минимум)

**Действия:**
1. Зарегистрировать компанию (см. раздел 5)
2. Настроить Paddle для приёма платежей
3. Развернуть TopGun Cloud на Hetzner (2 сервера, €30-60/мес)
4. **Show HN: "TopGun — Local-first real-time data platform in Rust"**
5. Открыть бету: Free tier для всех, Starter за $99 для ранних adopters
6. Discord-сервер для сообщества

**Цель:** 5-20 бесплатных пользователей, 2-5 платящих
**Доход: $200-500/мес**

### Фаза 2: Self-Serve Growth (через 3-6 месяцев после бета-запуска)

**Действия:**
1. Стабилизация cloud платформы
2. Self-service signup (без manual onboarding)
3. Добавить Pro-тариф ($299)
4. Контент-маркетинг: сравнения (TopGun vs Firebase, vs Supabase Realtime)
5. Первые conference talks (Rust-конференции, local-first meetups)

**Цель:** 20-50 платящих клиентов
**Доход: $3,000-10,000/мес**

### Фаза 3: Enterprise (через 12-18 месяцев после запуска)

**Триггер:** $5k+ MRR стабильно, есть запросы от enterprise

**Действия:**
1. Начать v3.0 enterprise фичи (multi-tenancy первым)
2. Enterprise тариф $999+/мес
3. Self-hosted Enterprise license
4. Первые enterprise контракты через inbound

**Цель:** $10-20k MRR, 3-5 enterprise клиентов
**Доход: $10,000-20,000/мес**

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

### Break-Even по тарифам

При base cost $60/мес + $3/tenant variable:

| Тариф | Клиентов до break-even | Клиентов до $5K MRR | Клиентов до $10K MRR |
|-------|----------------------|---------------------|---------------------|
| $99/мес | 1 | 52 | 105 |
| $299/мес | 1 | 17 | 34 |
| $999/мес | 1 | 5 | 10 |

**Реалистичный сценарий (60% Starter / 30% Pro / 10% Enterprise):**
- 20 клиентов → ~$5,000 MRR
- Infrastructure cost: $120/мес (2.4% от выручки)
- Paddle fees: $250/мес
- **Чистыми: ~$4,600/мес**

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

## 11. Конкретный план действий (следующие 30 дней)

### Параллельно с разработкой v2.0:

| # | Действие | Усилия | Важность |
|---|----------|--------|----------|
| 1 | Открыть Discord-сервер (text-only, без voice каналов) | 1 час | Высокая |
| 2 | Написать первый блог-пост: "Why I'm building a real-time data platform in Rust" (черновик RU → LLM → EN) | 4-8 часов | Высокая |
| 3 | Настроить waitlist на topgun.build (email-сбор) | 2-4 часа | Средняя |
| 4 | Начать Twitter/X build-in-public (короткие посты + скриншоты/гифки, минимум текста) | 30 мин/день | Средняя |
| 5 | Открыть Telegram-канал на русском для RU-сообщества | 30 мин | Средняя |
| 6 | Подготовить Stripe Atlas заявку (не отправлять до готовности cloud) | 1 час | Низкая |
| 7 | Начать Paddle регистрацию (занимает 1-2 недели) | 1 час | Низкая |
| 8 | Записаться на Italki/Preply (разговорный EN, 2-3 раза/неделю) | 1 час/setup | Средняя |

### После v2.0 feature-complete:

| # | Действие | Усилия | Важность |
|---|----------|--------|----------|
| 7 | Зарегистрировать компанию (Stripe Atlas) | $500 + 1 час | Критическая |
| 8 | Развернуть TopGun Cloud на Hetzner | 1-2 дня | Критическая |
| 9 | Show HN launch | 1 день | Критическая |
| 10 | Написать 2-3 comparison поста (SEO) | 1 неделя | Высокая |

---

## 12. Риски и митигация

| Риск | Вероятность | Митигация |
|------|------------|-----------|
| AWS/Cloudflare клонирует | Средняя | Скорость инновации + community moat + CRDT expertise |
| PlanetScale-сценарий (unit economics) | Низкая | Hetzner (3-5x дешевле AWS), высокая gross margin (97%) |
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
| Free tier users | 20 | 100 | 500 |
| Paying customers | 5 | 30 | 100 |
| MRR | $500 | $5,000 | $20,000 |
| Blog monthly visitors | 2,000 | 10,000 | 50,000 |

---

## Итого: Timeline to Exit

```
2026 Q2-Q3: Завершить v2.0, подготовить Cloud
2026 Q3-Q4: Бета-запуск, Show HN, первые платящие → $500-2K MRR
2027 Q1-Q2: Self-serve рост → $3-10K MRR
2027 Q3-Q4: Enterprise, v3.0 начало → $10-20K MRR
2028:       Развилка — lifestyle ($20-40K/мес) или seed round ($2-5M)
2029-2030:  При seed: $1-3M ARR → acquisition target ($10-30M+)
```

**Самый короткий путь к exit:** 3-4 года при агрессивном росте.
**Самый вероятный путь:** Прибыльный lifestyle business через 12-18 месяцев.
**Оба варианта хорошие** — решение принимать при достижении $10K MRR.
