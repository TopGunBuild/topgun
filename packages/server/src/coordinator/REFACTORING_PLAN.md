# ServerCoordinator Refactoring Plan

## Цели
- **< 800 строк** в `ServerCoordinator.ts`
- **Оптимальная когнитивная нагрузка** — каждый файл делает одну вещь
- **Чистая архитектура** — DI, single responsibility, testability

## Текущее состояние
- **ServerCoordinator.ts**: 1927 строк
- **Статус**: Код не компилируется (незавершённый рефакторинг)
- **Проблемы**: дублирование, несогласованность типов

## Целевая архитектура

```
ServerFactory (создание)
    ↓
ServerCoordinator (оркестрация, ~600-800 строк)
    ├── ConnectionHandler (WebSocket lifecycle)
    ├── LifecycleManager (startup/shutdown)
    ├── MessageRouter (message dispatch)
    └── [existing handlers in coordinator/]
```

---

## Phase 0: Исправить компиляцию (блокер)
**Цель**: Код компилируется
**Ожидаемый результат**: ~1750 строк

### Задачи:
- [ ] 0.1. Добавить `operationHandler: OperationHandler` в `ServerDependencies.ts`
- [ ] 0.2. В `ServerCoordinator` убрать создание `new OperationHandler({...})` (строки 463-478)
- [ ] 0.3. Использовать `dependencies.operationHandler` вместо `this.operationHandler`
- [ ] 0.4. Удалить дублированный метод `applyOpToMap` (строки 1441-1573) — **-132 строки**
- [ ] 0.5. Удалить дублированный метод `processLocalOp` (строки 1635-1684) — **-50 строки**
- [ ] 0.6. Обновить все вызовы `this.applyOpToMap` → `this.operationHandler.applyOpToMap`
- [ ] 0.7. Запустить `pnpm exec tsc --noEmit` — должен компилироваться

---

## Phase 1: Извлечь Connection Handling
**Цель**: Отделить WebSocket lifecycle от координации
**Ожидаемый результат**: ~1550 строк

### Новый файл: `coordinator/connection-handler.ts`
```typescript
export class ConnectionHandler {
    constructor(config: ConnectionHandlerConfig) {}

    handleConnection(ws: WebSocket): void
    handleMessage(client: ClientConnection, rawMessage: any): Promise<void>
}
```

### Задачи:
- [ ] 1.1. Создать `ConnectionHandlerConfig` в `types.ts`
- [ ] 1.2. Извлечь `handleConnection` (строки 1123-1276) — **-153 строки**
- [ ] 1.3. Извлечь `handleMessage` (строки 1278-1341) — **-63 строки**
- [ ] 1.4. Добавить в `ServerFactory` создание `ConnectionHandler`
- [ ] 1.5. В `ServerCoordinator` делегировать: `this.wss.on('connection', (ws) => this.connectionHandler.handleConnection(ws))`

---

## Phase 2: Извлечь Lifecycle Management
**Цель**: Отделить startup/shutdown логику
**Ожидаемый результат**: ~1350 строк

### Новый файл: `coordinator/lifecycle-manager.ts`
```typescript
export class LifecycleManager {
    constructor(config: LifecycleManagerConfig) {}

    async shutdown(): Promise<void>
    async gracefulClusterDeparture(): Promise<void>
    async backfillSearchIndexes(): Promise<void>
}
```

### Задачи:
- [ ] 2.1. Создать `LifecycleManagerConfig` в `types.ts`
- [ ] 2.2. Извлечь `shutdown` (строки 997-1121) — **-124 строки**
- [ ] 2.3. Извлечь `gracefulClusterDeparture` (строки 907-958) — **-51 строка**
- [ ] 2.4. Извлечь `waitForReplicationFlush` (строки 981-995) — **-14 строк**
- [ ] 2.5. Извлечь `backfillSearchIndexes` (строки 727-755) — **-28 строк**
- [ ] 2.6. В `ServerCoordinator.shutdown()` делегировать в `lifecycleManager`

---

## Phase 3: Консолидировать Public API
**Цель**: Убрать trivial delegations, оставить только facade методы
**Ожидаемый результат**: ~1150 строк

### Задачи:
- [ ] 3.1. Metrics API — сделать `metricsService` публичным свойством вместо 10+ геттеров
  - Удалить: `getEventExecutorMetrics`, `getEventExecutorTotalMetrics`, `getRateLimiterStats`, etc.
  - Пользователь вызывает: `coordinator.metrics.getEventExecutorMetrics()`
  - **-80 строк**

- [ ] 3.2. FTS API — делегировать напрямую в `searchCoordinator`
  - Сделать `searchCoordinator` публичным
  - Удалить обёртки: `enableFullTextSearch`, `disableFullTextSearch`, etc.
  - **-60 строк**

- [ ] 3.3. Storage API — `storageManager` уже публичный, удалить обёртки
  - `getMap` → `storageManager.getMap`
  - `getMapAsync` → `storageManager.getMapAsync`
  - **-20 строк**

---

## Phase 4: Упростить Constructor
**Цель**: Constructor только присваивает dependencies, wiring в отдельном методе
**Ожидаемый результат**: ~950 строк

### Задачи:
- [ ] 4.1. Переместить весь event listener setup в отдельный метод `setupEventListeners()`
- [ ] 4.2. Переместить handler wiring в `setupHandlerWiring()`
- [ ] 4.3. Упростить constructor до:
  ```typescript
  constructor(config, dependencies) {
      this.assignDependencies(dependencies);
      this.setupEventListeners();
      this.setupHandlerWiring();
      this.startServices();
  }
  ```
- [ ] 4.4. **Альтернатива**: Перенести wiring в `ServerFactory` полностью
  - Factory вызывает `coordinator.onReady()` после создания
  - **-200 строк** из constructor

---

## Phase 5: Удалить мёртвый код и оптимизировать
**Цель**: Финальная чистка
**Ожидаемый результат**: ~800 строк

### Задачи:
- [ ] 5.1. Удалить неиспользуемые private методы
- [ ] 5.2. Inline однострочные делегации
- [ ] 5.3. Удалить `buildTLSOptions` (уже есть в `ServerFactory`)
- [ ] 5.4. Переместить `PendingClusterQuery` interface в `types.ts`
- [ ] 5.5. Переместить `ServerCoordinatorConfig` в отдельный файл `config.ts`
- [ ] 5.6. Консолидировать imports

---

## Phase 6: Валидация архитектуры
**Цель**: Убедиться в качестве

### Чеклист:
- [ ] Все тесты проходят: `pnpm --filter @topgunbuild/server test`
- [ ] TypeScript компилируется без ошибок
- [ ] Каждый файл < 500 строк (кроме types.ts)
- [ ] Cyclomatic complexity < 10 для каждого метода
- [ ] Нет circular dependencies

---

## Итоговая структура файлов

```
packages/server/src/
├── ServerCoordinator.ts        (~600-800 строк) - оркестратор
├── ServerFactory.ts            (~700 строк) - DI container
├── ServerDependencies.ts       (~120 строк) - интерфейс зависимостей
├── config.ts                   (~100 строк) - ServerCoordinatorConfig
└── coordinator/
    ├── index.ts
    ├── types.ts                (~1000 строк) - все интерфейсы
    ├── connection-handler.ts   (~250 строк) - WebSocket lifecycle
    ├── lifecycle-manager.ts    (~220 строк) - startup/shutdown
    ├── operation-handler.ts    (~285 строк) - CRDT operations
    ├── broadcast-handler.ts    (~150 строк) - event broadcast
    ├── batch-processing-handler.ts
    ├── write-concern-handler.ts
    ├── heartbeat-handler.ts
    ├── gc-handler.ts
    ├── ... (other handlers)
    └── message-registry.ts
```

---

## Оценка времени

| Phase | Сложность | Риск | Строки удалить |
|-------|-----------|------|----------------|
| 0     | Низкая    | Низкий | ~180 |
| 1     | Средняя   | Средний | ~216 |
| 2     | Средняя   | Средний | ~217 |
| 3     | Низкая    | Низкий | ~160 |
| 4     | Средняя   | Средний | ~200 |
| 5     | Низкая    | Низкий | ~150 |

**Итого**: ~1123 строки → результат ~804 строки

---

## Когнитивная нагрузка: До vs После

### До (проблемы):
- 1927 строк в одном файле
- 61 метод с разной ответственностью
- Смешение: connection handling + CRDT ops + lifecycle + metrics + FTS
- Дублирование кода
- Сложно понять что происходит

### После (улучшения):
- ~700 строк — координация и делегирование
- Каждый handler: одна ответственность
- Чёткие границы: connection → message → operation → broadcast
- Легко найти код: "где обрабатывается shutdown?" → `lifecycle-manager.ts`
- Легко тестировать каждый компонент отдельно

---

## Следующий шаг

Начать с **Phase 0** — исправить компиляцию. Это блокер для всего остального.
