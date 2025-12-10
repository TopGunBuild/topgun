# Push Worker

Cloudflare Worker для отправки Web Push уведомлений. Используется вместе с n8n для отправки напоминаний из Notes App.

## Архитектура

```
┌──────────────┐     TopGun Sync      ┌──────────────┐
│   PWA        │ ──────────────────→  │  PostgreSQL  │
│  (Client)    │                      │              │
└──────────────┘                      └──────┬───────┘
       │                                     │
       │ GET /api/vapid-public-key           │ SQL Query
       ▼                                     ▼
┌──────────────┐                      ┌──────────────┐
│ Push Worker  │ ◄──────────────────  │     n8n      │
│ (Cloudflare) │   POST /api/push/    │   (Cron)     │
└──────┬───────┘                      └──────────────┘
       │
       │ Web Push Protocol (encrypted)
       ▼
┌──────────────┐
│ Push Service │
│ (FCM/Mozilla)│
└──────────────┘
```

## Установка

### 1. Генерация VAPID ключей

```bash
cd examples/push-worker
npm install
npm run generate-vapid
```

Вывод:
```
🔑 VAPID Keys Generated

Public Key (use in client & wrangler secret):
BNxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

Private Key (use in wrangler secret only):
xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

### 2. Настройка secrets в Cloudflare

```bash
# Установить публичный ключ
echo "BNxxxx..." | wrangler secret put VAPID_PUBLIC_KEY

# Установить приватный ключ (НИКОГДА не шарить!)
echo "xxxxx..." | wrangler secret put VAPID_PRIVATE_KEY
```

### 3. Настройка wrangler.toml

```toml
name = "push-worker"
main = "src/index.ts"
compatibility_date = "2024-01-01"
account_id = "your-account-id"

[vars]
VAPID_SUBJECT = "mailto:admin@your-domain.com"
ALLOWED_ORIGIN = "https://your-notes-app.com"
```

### 4. Деплой

```bash
npm run deploy
```

## API Endpoints

### GET /api/vapid-public-key

Возвращает публичный VAPID ключ для клиентской подписки.

**Response:**
```json
{
  "publicKey": "BNxxxxxxxxxxxxxxxx..."
}
```

### POST /api/push/send

Отправляет одно push-уведомление. Вызывается из n8n.

**Request:**
```json
{
  "subscription": {
    "endpoint": "https://fcm.googleapis.com/fcm/send/xxx...",
    "keys": {
      "p256dh": "BNxxxxxxxx...",
      "auth": "xxxxxxxx..."
    }
  },
  "payload": {
    "title": "Напоминание",
    "body": "Пора проверить заметку",
    "icon": "/icon-192.svg",
    "data": {
      "noteId": "abc123",
      "url": "/?note=abc123"
    }
  },
  "ttl": 86400
}
```

**Response (success):**
```json
{
  "success": true,
  "statusCode": 201,
  "endpoint": "https://fcm.googleapis.com/..."
}
```

**Response (expired subscription):**
```json
{
  "success": false,
  "statusCode": 410,
  "error": "Subscription expired or invalid",
  "endpoint": "..."
}
```

### POST /api/push/send-batch

Отправляет несколько уведомлений за один запрос.

**Request:**
```json
{
  "notifications": [
    {
      "subscription": { "endpoint": "...", "keys": { "p256dh": "...", "auth": "..." } },
      "payload": { "title": "...", "body": "..." }
    },
    {
      "subscription": { "endpoint": "...", "keys": { "p256dh": "...", "auth": "..." } },
      "payload": { "title": "...", "body": "..." }
    }
  ]
}
```

**Response:**
```json
{
  "total": 2,
  "success": 2,
  "failed": 0,
  "results": [
    { "success": true, "statusCode": 201 },
    { "success": true, "statusCode": 201 }
  ]
}
```

### GET /health

Health check endpoint.

```json
{
  "status": "ok",
  "timestamp": 1699999999999
}
```

## Настройка Notes App

### 1. Добавить переменные окружения

В `.env` файл notes-app:

```env
VITE_PUSH_WORKER_URL=https://push-worker.your-account.workers.dev
VITE_VAPID_PUBLIC_KEY=BNxxxxxxxxxxxxxxxx...
```

### 2. Использование в коде

```typescript
import {
  isPushSupported,
  subscribeToPush,
  unsubscribeFromPush,
  isSubscribed,
  getPermissionStatus
} from './lib/pushNotifications';

// Проверить поддержку
if (isPushSupported()) {
  // Подписаться
  const subscription = await subscribeToPush(userId);

  // Проверить статус
  const subscribed = await isSubscribed();

  // Отписаться
  await unsubscribeFromPush(userId);
}
```

## Настройка n8n

### Workflow: Scheduled Notes Reminder

```
[Cron: * * * * *] → [PostgreSQL] → [Loop] → [HTTP Request] → [PostgreSQL Update]
```

### 1. Cron Trigger

- Тип: Cron
- Expression: `* * * * *` (каждую минуту)

### 2. PostgreSQL: Get Due Notes

```sql
SELECT
  n.data->>'id' as note_id,
  n.data->>'title' as title,
  n.data->>'userId' as user_id,
  n.data->>'date' as due_date,
  n.data->>'time' as due_time,
  p.data->>'endpoint' as endpoint,
  p.data->>'p256dh' as p256dh,
  p.data->>'auth' as auth
FROM topgun_nodes n
JOIN topgun_nodes p ON p.data->>'userId' = n.data->>'userId'
WHERE
  n.soul LIKE 'notes/%'
  AND p.soul LIKE 'pushSubscriptions/%'
  AND n.data->>'date' = CURRENT_DATE::text
  AND n.data->>'time' = TO_CHAR(NOW(), 'HH24:MI')
  AND COALESCE((n.data->>'notified')::boolean, false) = false;
```

### 3. HTTP Request: Send Push

- Method: POST
- URL: `https://push-worker.xxx.workers.dev/api/push/send`
- Body:
```json
{
  "subscription": {
    "endpoint": "{{ $json.endpoint }}",
    "keys": {
      "p256dh": "{{ $json.p256dh }}",
      "auth": "{{ $json.auth }}"
    }
  },
  "payload": {
    "title": "Напоминание",
    "body": "{{ $json.title }}",
    "data": {
      "noteId": "{{ $json.note_id }}"
    }
  }
}
```

### 4. PostgreSQL: Mark as Notified

```sql
UPDATE topgun_nodes
SET data = jsonb_set(data, '{notified}', 'true')
WHERE soul LIKE 'notes/%' AND data->>'id' = '{{ $json.note_id }}';
```

## Структура данных в TopGun/PostgreSQL

### Push Subscriptions

Path: `pushSubscriptions/{userId}/{deviceId}`

```json
{
  "deviceId": "uuid-xxx",
  "userId": "clerk_user_xxx",
  "endpoint": "https://fcm.googleapis.com/fcm/send/xxx",
  "p256dh": "BNxxxxxxxx...",
  "auth": "xxxxxxxx...",
  "createdAt": 1699999999999,
  "userAgent": "Mozilla/5.0..."
}
```

### Notes with Schedule

Path: `notes/{userId}/{noteId}`

```json
{
  "id": "note-xxx",
  "title": "Важная встреча",
  "content": "...",
  "date": "2024-01-15",
  "time": "10:00",
  "recurring": "weekly",
  "notified": false
}
```

## Обработка ошибок

| Status Code | Значение | Действие в n8n |
|-------------|----------|----------------|
| 201 | Успешно отправлено | Пометить как notified |
| 410 | Подписка истекла | Удалить подписку из БД |
| 404 | Подписка не найдена | Удалить подписку из БД |
| 429 | Rate limit | Повторить позже |
| 500 | Ошибка сервера | Логировать, повторить |

## Безопасность

- VAPID Private Key хранится только в Cloudflare Secrets
- CORS ограничен через ALLOWED_ORIGIN
- Payload шифруется по RFC 8291 (aes128gcm)
- Каждое устройство имеет уникальные ключи шифрования
