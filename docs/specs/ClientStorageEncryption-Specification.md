# Client Storage Encryption — Техническая спецификация

**Версия:** 1.0
**Дата:** 2025-12-06
**Статус:** Draft

## 1. Обзор

### 1.1 Цель
Обеспечить защиту данных пользователя "в покое" (Data At Rest) на клиентском устройстве (Browser). Это критично для Offline-First приложений, где данные хранятся локально в IndexedDB продолжительное время.

### 1.2 Решение
Реализовать "прозрачное" шифрование на уровне Storage Adapter.
Приложение будет работать с `EncryptedStorageAdapter`, который оборачивает реальный адаптер (например `IDBAdapter`) и шифрует/расшифровывает данные "на лету" с использованием WebCrypto API (AES-GCM).

## 2. Архитектура

### 2.1 Encryption Layer
```mermaid
graph TD
    ClientApp[TopGun Client] -->|put(key, cleartext)| EncAdapter[EncryptedStorageAdapter]
    EncAdapter -->|Encrypt (AES-GCM)| IDBAdapter[IDBAdapter]
    IDBAdapter -->|store(iv + ciphertext)| IndexedDB[(Browser IndexedDB)]
```

### 2.2 Структура хранения

Вместо хранения чистого значения `{ key: "foo", value: { "bar": 123 } }`, мы будем хранить структуру **EncryptedRecord**:

```typescript
interface EncryptedRecord {
  iv: Uint8Array;       // 12 bytes initialization vector
  data: Uint8Array;     // Ciphertext (AES-GCM-256)
}
```

## 3. Key Management

### 3.1 Ключи
Мы будем использовать **AES-256** (32 bytes).

### 3.2 Источник ключа
Адаптер не занимается *созданием* ключа, он принимает уже готовый `CryptoKey`. Приложение должно предоставить ключ при инициализации.

**Варианты получения ключа (на уровне приложения):**
1.  **Passphrase-based**: Пользователь вводит пароль -> PBKDF2 -> AES Key.
2.  **Server-provided**: При входе сервер отдаёт ключ (через TLS), клиент сохраняет его в памяти (SessionStorage).
3.  **Generated**: Клиент генерирует случайный ключ и хранит его в локальном хранилище (менее безопасно, но защищает от дампов диска).

*В рамках этой спецификации мы реализуем только механизм шифрования по предоставленному ключу.*

## 4. API Изменения

### 4.1 Новый класс `EncryptedStorageAdapter`

```typescript
// packages/client/src/adapters/EncryptedStorageAdapter.ts

import { IStorageAdapter, OpLogEntry } from '../IStorageAdapter';

export class EncryptedStorageAdapter implements IStorageAdapter {
  constructor(
    private wrapped: IStorageAdapter,
    private key: CryptoKey
  ) {}

  // ... implementation ...
}
```

### 4.2 Поддержка CryptoUtils

```typescript
// packages/client/src/crypto/EncryptionManager.ts (or CryptoUtils.ts)

export class CryptoUtils {
  static async encrypt(key: CryptoKey, data: any): Promise<{ iv: Uint8Array, data: Uint8Array }> {
    // 1. Serialize data (JSON or MsgPack) -> Uint8Array
    // 2. Generate IV (12 bytes)
    // 3. window.crypto.subtle.encrypt(AES-GCM, iv, data)
    // 4. Return { iv, ciphertext }
  }

  static async decrypt(key: CryptoKey, record: { iv: Uint8Array, data: Uint8Array }): Promise<any> {
    // 1. window.crypto.subtle.decrypt(AES-GCM, iv, ciphertext)
    // 2. Deserialize -> Object
  }
}
```

## 5. Детали реализации

### 5.1 Метод `get(key)`
1.  Вызвать `wrapped.get(key)`.
2.  Если вернулось `null/undefined` -> вернуть `null/undefined`.
3.  Проверить, является ли объект зашифрованным (Duck typing: есть поля `iv` и `data`).
    *   *Примечание*: Для обратной совместимости, если данные не зашифрованы, можно возвращать как есть (Migration strategy). Или строго фейлить. Для начала - строго.
4.  Расшифровать: `CryptoUtils.decrypt(key, record)`.
5.  Вернуть расшифрованные данные.

### 5.2 Метод `put(key, value)`
1.  Зашифровать: `ciphertext = CryptoUtils.encrypt(key, value)`.
2.  Вызвать `wrapped.put(key, ciphertext)`.

### 5.3 OpLog (`appendOpLog`)
**Проблема**: OpLog содержит сами данные операций (`value`, `record`).
**Решение**: Шифровать поля `value` и `record` внутри `OpLogEntry` перед сохранением.

*   `OpLogEntry.key` - **не шифруем** (нужен для индексации/поиска).
*   `OpLogEntry.mapName` - **не шифруем** (метаданные).
*   `OpLogEntry.record` (весь объект) -> сериализуем и шифруем.

### 5.4 Метаданные (`getMeta`, `setMeta`)
Метаданные (например HLC clocks, sync cursors) обычно не чувствительны.
**Решение**: Шифруем `setMeta` так же, как и `put`.

## 6. Тестирование

### 6.1 Unit Tests
*   `CryptoUtils.test.ts`: Проверка encrypt/decrypt roundtrip.
*   `EncryptedStorageAdapter.test.ts`:
    *   Mock `wrapped` adapter (InMemory).
    *   Проверка, что в `wrapped` попадают *другие* байты, не совпадающие с исходными.
    *   Проверка, что `get` возвращает исходные данные.

### 6.2 Browser Tests (если возможно)
*   Использование `jsdom` с полифиллом `webcrypto` (Node.js 19+ поддерживает `globalThis.crypto`).

## 7. Migration (Future Work)
Как мигрировать существующую базу данных без шифрования на шифрование?
*   Понадобится утилита, которая:
    1.  Читает все ключи.
    2.  Для каждого ключа: читает plaintext, шифрует, переписывает.
*   Это выходит за рамки Phase 2, так как сейчас мы фокусируемся на *новой* установке или просто включении шифрования (с потерей старых данных, если они были).

---
