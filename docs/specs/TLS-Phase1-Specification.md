# TLS Phase 1: Transport Layer Security — Техническая спецификация

**Версия:** 1.0
**Дата:** 2025-12-06
**Статус:** Draft
**Автор:** Claude (AI Assistant)

---

## 1. Обзор

### 1.1 Цель

Внедрение TLS (Transport Layer Security) для защиты всех сетевых коммуникаций в TopGun:

- **Client-Server:** Переход с `ws://` на `wss://` (WebSocket Secure)
- **Cluster (Node-to-Node):** Переход с `ws://` на `wss://` с поддержкой mTLS
- **Metrics Endpoint:** Переход с `http://` на `https://`

### 1.2 Scope

| Компонент | Текущее состояние | Целевое состояние |
|-----------|-------------------|-------------------|
| Client WebSocket | `ws://` | `wss://` |
| Cluster WebSocket | `ws://` | `wss://` + mTLS (опционально) |
| Metrics HTTP | `http://` | `https://` (опционально) |

### 1.3 Не входит в scope

- Шифрование данных at-rest (Phase 2)
- Field-Level Encryption (Phase 3)
- Key rotation механизмы (Phase 3)

---

## 2. Архитектура

### 2.1 Высокоуровневая диаграмма

```
┌─────────────────────────────────────────────────────────────────────┐
│                        PRODUCTION ENVIRONMENT                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│    ┌──────────┐          wss://           ┌──────────────────┐      │
│    │  Client  │ ◄──────────────────────► │   TopGun Node 1  │      │
│    │ (Browser)│         TLS 1.3          │                  │      │
│    └──────────┘                          │  ┌────────────┐  │      │
│                                          │  │ HTTPS Srv  │  │      │
│    ┌──────────┐          wss://          │  └────────────┘  │      │
│    │  Client  │ ◄──────────────────────► │                  │      │
│    │  (Node)  │         TLS 1.3          └────────┬─────────┘      │
│    └──────────┘                                   │                 │
│                                                   │ wss:// + mTLS   │
│                                          ┌────────▼─────────┐      │
│                                          │   TopGun Node 2  │      │
│                                          │                  │      │
│                                          └──────────────────┘      │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### 2.2 Конфигурационная модель

```typescript
// packages/server/src/types/TLSConfig.ts

export interface TLSConfig {
  /**
   * Включить TLS для client-facing сервера (HTTPS + WSS)
   * @default false
   */
  enabled: boolean;

  /**
   * Путь к файлу сертификата (PEM format)
   * Поддерживает chain certificates
   */
  certPath: string;

  /**
   * Путь к файлу приватного ключа (PEM format)
   */
  keyPath: string;

  /**
   * Путь к CA certificate для проверки клиентских сертификатов
   * Требуется для mTLS
   * @optional
   */
  caCertPath?: string;

  /**
   * Минимальная версия TLS
   * @default 'TLSv1.2'
   */
  minVersion?: 'TLSv1.2' | 'TLSv1.3';

  /**
   * Список разрешённых cipher suites
   * @optional - использовать Node.js defaults если не указано
   */
  ciphers?: string;

  /**
   * Passphrase для зашифрованного приватного ключа
   * @optional
   */
  passphrase?: string;
}

export interface ClusterTLSConfig extends TLSConfig {
  /**
   * Требовать клиентский сертификат (mTLS)
   * @default false
   */
  requireClientCert?: boolean;

  /**
   * Проверять сертификат пиров
   * В development может быть отключено для self-signed certs
   * @default true
   */
  rejectUnauthorized?: boolean;
}
```

---

## 3. Изменения в коде

### 3.1 ServerCoordinator.ts

#### 3.1.1 Текущий код (строки 111-143)

```typescript
// BEFORE: Plain HTTP
this.httpServer = createServer((_req, res) => {
    res.writeHead(200);
    res.end('TopGun Server Running');
});
// ...
this.wss = new WebSocketServer({ server: this.httpServer });
```

#### 3.1.2 Целевой код

```typescript
// packages/server/src/ServerCoordinator.ts

import { createServer as createHttpServer, Server as HttpServer } from 'http';
import { createServer as createHttpsServer, Server as HttpsServer } from 'https';
import { readFileSync } from 'fs';
import { TLSConfig } from './types/TLSConfig';

export interface ServerCoordinatorConfig {
    port: number;
    nodeId: string;
    storage?: IServerStorage;
    jwtSecret?: string;
    host?: string;
    clusterPort?: number;
    peers?: string[];
    securityPolicies?: PermissionPolicy[];
    resolvePeers?: () => string[];
    interceptors?: IInterceptor[];
    metricsPort?: number;
    discovery?: 'manual' | 'kubernetes';
    serviceName?: string;
    discoveryInterval?: number;

    // NEW: TLS Configuration
    tls?: TLSConfig;
    clusterTls?: ClusterTLSConfig;
}

// В конструкторе:
constructor(config: ServerCoordinatorConfig) {
    // ... existing code ...

    // HTTP/HTTPS Server Setup
    if (config.tls?.enabled) {
        const tlsOptions = this.buildTLSOptions(config.tls);
        this.httpServer = createHttpsServer(tlsOptions, (_req, res) => {
            res.writeHead(200);
            res.end('TopGun Server Running (Secure)');
        });
        logger.info('TLS enabled for client connections');
    } else {
        this.httpServer = createHttpServer((_req, res) => {
            res.writeHead(200);
            res.end('TopGun Server Running');
        });

        // Warning in production
        if (process.env.NODE_ENV === 'production') {
            logger.warn('⚠️  TLS is disabled! Client connections are NOT encrypted.');
        }
    }

    // WebSocket Server (automatically uses wss:// if underlying server is HTTPS)
    this.wss = new WebSocketServer({ server: this.httpServer });

    // ... rest of constructor ...
}

private buildTLSOptions(config: TLSConfig): https.ServerOptions {
    const options: https.ServerOptions = {
        cert: readFileSync(config.certPath),
        key: readFileSync(config.keyPath),
        minVersion: config.minVersion || 'TLSv1.2',
    };

    if (config.caCertPath) {
        options.ca = readFileSync(config.caCertPath);
    }

    if (config.ciphers) {
        options.ciphers = config.ciphers;
    }

    if (config.passphrase) {
        options.passphrase = config.passphrase;
    }

    return options;
}
```

### 3.2 ClusterManager.ts

#### 3.2.1 Текущий код (строка 189)

```typescript
// BEFORE: Plain WebSocket
const ws = new WebSocket(`ws://${peerAddress}`);
```

#### 3.2.2 Целевой код

```typescript
// packages/server/src/cluster/ClusterManager.ts

import { readFileSync } from 'fs';
import * as https from 'https';
import { ClusterTLSConfig } from '../types/TLSConfig';

export interface ClusterConfig {
  nodeId: string;
  host: string;
  port: number;
  peers: string[];
  discovery?: 'manual' | 'kubernetes';
  serviceName?: string;
  discoveryInterval?: number;

  // NEW: TLS Configuration
  tls?: ClusterTLSConfig;
}

// В классе ClusterManager:

public start(): Promise<number> {
    return new Promise((resolve) => {
        logger.info({ port: this.config.port, tls: !!this.config.tls?.enabled }, 'Starting Cluster Manager');

        if (this.config.tls?.enabled) {
            // HTTPS-based WebSocket Server for cluster
            const tlsOptions = this.buildClusterTLSOptions();
            const httpsServer = https.createServer(tlsOptions);
            this.server = new WebSocketServer({ server: httpsServer });

            httpsServer.listen(this.config.port, () => {
                const addr = httpsServer.address();
                this._actualPort = typeof addr === 'object' && addr ? addr.port : this.config.port;
                logger.info({ port: this._actualPort }, 'Cluster Manager listening (TLS enabled)');
                this.onServerReady(resolve);
            });
        } else {
            // Plain WebSocket Server (development only)
            this.server = new WebSocketServer({ port: this.config.port });

            this.server.on('listening', () => {
                const addr = this.server!.address();
                this._actualPort = typeof addr === 'object' && addr ? addr.port : this.config.port;
                logger.info({ port: this._actualPort }, 'Cluster Manager listening');
                this.onServerReady(resolve);
            });

            if (process.env.NODE_ENV === 'production') {
                logger.warn('⚠️  Cluster TLS is disabled! Inter-node traffic is NOT encrypted.');
            }
        }

        // ... rest of start() ...
    });
}

private _connectToPeerInternal(peerAddress: string, attempt: number) {
    // ... existing checks ...

    logger.info({ peerAddress, attempt, tls: !!this.config.tls?.enabled }, 'Connecting to peer');
    this.pendingConnections.add(peerAddress);

    try {
        let ws: WebSocket;

        if (this.config.tls?.enabled) {
            // Secure WebSocket connection
            const protocol = 'wss://';
            const wsOptions: WebSocket.ClientOptions = {
                rejectUnauthorized: this.config.tls.rejectUnauthorized !== false,
            };

            // mTLS: Provide client certificate
            if (this.config.tls.certPath && this.config.tls.keyPath) {
                wsOptions.cert = readFileSync(this.config.tls.certPath);
                wsOptions.key = readFileSync(this.config.tls.keyPath);

                if (this.config.tls.passphrase) {
                    wsOptions.passphrase = this.config.tls.passphrase;
                }
            }

            // CA for peer verification
            if (this.config.tls.caCertPath) {
                wsOptions.ca = readFileSync(this.config.tls.caCertPath);
            }

            ws = new WebSocket(`${protocol}${peerAddress}`, wsOptions);
        } else {
            // Plain WebSocket (development)
            ws = new WebSocket(`ws://${peerAddress}`);
        }

        // ... rest of connection handling ...
    } catch (e) {
        // ... error handling ...
    }
}

private buildClusterTLSOptions(): https.ServerOptions {
    const config = this.config.tls!;

    const options: https.ServerOptions = {
        cert: readFileSync(config.certPath),
        key: readFileSync(config.keyPath),
        minVersion: config.minVersion || 'TLSv1.2',
    };

    if (config.caCertPath) {
        options.ca = readFileSync(config.caCertPath);
    }

    if (config.requireClientCert) {
        options.requestCert = true;
        options.rejectUnauthorized = true;
    }

    if (config.passphrase) {
        options.passphrase = config.passphrase;
    }

    return options;
}
```

### 3.3 start-server.ts

#### 3.3.1 Обновлённая конфигурация

```typescript
// packages/server/src/start-server.ts

import { ServerCoordinator } from './ServerCoordinator';
import { PostgresAdapter } from './storage/PostgresAdapter';
import { logger } from './utils/logger';
import { TLSConfig, ClusterTLSConfig } from './types/TLSConfig';

// Existing Configuration
const PORT = parseInt(process.env.TOPGUN_PORT || '8080', 10);
const CLUSTER_PORT = parseInt(process.env.TOPGUN_CLUSTER_PORT || '9080', 10);
const NODE_ID = process.env.NODE_ID || `node-${Math.random().toString(36).substring(2, 8)}`;
const PEERS = process.env.TOPGUN_PEERS ? process.env.TOPGUN_PEERS.split(',') : [];
const DISCOVERY_SERVICE = process.env.TOPGUN_DISCOVERY_SERVICE;
const DISCOVERY_INTERVAL = parseInt(process.env.TOPGUN_DISCOVERY_INTERVAL || '10000', 10);
const DISCOVERY_MODE = DISCOVERY_SERVICE ? 'kubernetes' : 'manual';
const DB_URL = process.env.DATABASE_URL;

// NEW: TLS Configuration
const TLS_ENABLED = process.env.TOPGUN_TLS_ENABLED === 'true';
const TLS_CERT_PATH = process.env.TOPGUN_TLS_CERT_PATH;
const TLS_KEY_PATH = process.env.TOPGUN_TLS_KEY_PATH;
const TLS_CA_PATH = process.env.TOPGUN_TLS_CA_PATH;
const TLS_MIN_VERSION = (process.env.TOPGUN_TLS_MIN_VERSION as 'TLSv1.2' | 'TLSv1.3') || 'TLSv1.2';
const TLS_PASSPHRASE = process.env.TOPGUN_TLS_PASSPHRASE;

// Cluster TLS (can use same certs or separate)
const CLUSTER_TLS_ENABLED = process.env.TOPGUN_CLUSTER_TLS_ENABLED === 'true';
const CLUSTER_TLS_CERT_PATH = process.env.TOPGUN_CLUSTER_TLS_CERT_PATH || TLS_CERT_PATH;
const CLUSTER_TLS_KEY_PATH = process.env.TOPGUN_CLUSTER_TLS_KEY_PATH || TLS_KEY_PATH;
const CLUSTER_TLS_CA_PATH = process.env.TOPGUN_CLUSTER_TLS_CA_PATH || TLS_CA_PATH;
const CLUSTER_TLS_REQUIRE_CLIENT_CERT = process.env.TOPGUN_CLUSTER_MTLS === 'true';
const CLUSTER_TLS_REJECT_UNAUTHORIZED = process.env.TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED !== 'false';

// Build TLS Config
let tlsConfig: TLSConfig | undefined;
if (TLS_ENABLED) {
    if (!TLS_CERT_PATH || !TLS_KEY_PATH) {
        logger.error('TLS is enabled but TOPGUN_TLS_CERT_PATH or TOPGUN_TLS_KEY_PATH is missing');
        process.exit(1);
    }

    tlsConfig = {
        enabled: true,
        certPath: TLS_CERT_PATH,
        keyPath: TLS_KEY_PATH,
        caCertPath: TLS_CA_PATH,
        minVersion: TLS_MIN_VERSION,
        passphrase: TLS_PASSPHRASE,
    };

    logger.info({ certPath: TLS_CERT_PATH, minVersion: TLS_MIN_VERSION }, 'Client TLS configured');
}

// Build Cluster TLS Config
let clusterTlsConfig: ClusterTLSConfig | undefined;
if (CLUSTER_TLS_ENABLED) {
    if (!CLUSTER_TLS_CERT_PATH || !CLUSTER_TLS_KEY_PATH) {
        logger.error('Cluster TLS is enabled but cert/key paths are missing');
        process.exit(1);
    }

    clusterTlsConfig = {
        enabled: true,
        certPath: CLUSTER_TLS_CERT_PATH,
        keyPath: CLUSTER_TLS_KEY_PATH,
        caCertPath: CLUSTER_TLS_CA_PATH,
        minVersion: TLS_MIN_VERSION,
        passphrase: TLS_PASSPHRASE,
        requireClientCert: CLUSTER_TLS_REQUIRE_CLIENT_CERT,
        rejectUnauthorized: CLUSTER_TLS_REJECT_UNAUTHORIZED,
    };

    logger.info({
        certPath: CLUSTER_TLS_CERT_PATH,
        mTLS: CLUSTER_TLS_REQUIRE_CLIENT_CERT
    }, 'Cluster TLS configured');
}

// Setup Storage
let storage;
if (DB_URL) {
    storage = new PostgresAdapter({ connectionString: DB_URL });
    logger.info('Using PostgresAdapter with DATABASE_URL');
} else {
    logger.info('No DATABASE_URL provided, using in-memory storage (non-persistent)');
}

const server = new ServerCoordinator({
    port: PORT,
    clusterPort: CLUSTER_PORT,
    nodeId: NODE_ID,
    peers: PEERS,
    discovery: DISCOVERY_MODE,
    serviceName: DISCOVERY_SERVICE,
    discoveryInterval: DISCOVERY_INTERVAL,
    storage,
    host: '0.0.0.0',
    securityPolicies: [
        {
            role: 'USER',
            mapNamePattern: '*',
            actions: ['ALL']
        }
    ],

    // NEW: TLS
    tls: tlsConfig,
    clusterTls: clusterTlsConfig,
});

// ... rest of file unchanged ...
```

---

## 4. Переменные окружения

### 4.1 Полный список новых переменных

| Переменная | Тип | Default | Описание |
|------------|-----|---------|----------|
| `TOPGUN_TLS_ENABLED` | boolean | `false` | Включить TLS для клиентских соединений |
| `TOPGUN_TLS_CERT_PATH` | string | - | Путь к сертификату (PEM) |
| `TOPGUN_TLS_KEY_PATH` | string | - | Путь к приватному ключу (PEM) |
| `TOPGUN_TLS_CA_PATH` | string | - | Путь к CA certificate (опционально) |
| `TOPGUN_TLS_MIN_VERSION` | enum | `TLSv1.2` | Минимальная версия TLS |
| `TOPGUN_TLS_PASSPHRASE` | string | - | Passphrase для зашифрованного ключа |
| `TOPGUN_CLUSTER_TLS_ENABLED` | boolean | `false` | Включить TLS для cluster |
| `TOPGUN_CLUSTER_TLS_CERT_PATH` | string | - | Путь к cluster сертификату |
| `TOPGUN_CLUSTER_TLS_KEY_PATH` | string | - | Путь к cluster ключу |
| `TOPGUN_CLUSTER_TLS_CA_PATH` | string | - | Путь к CA для проверки пиров |
| `TOPGUN_CLUSTER_MTLS` | boolean | `false` | Требовать клиентский сертификат |
| `TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED` | boolean | `true` | Проверять сертификаты пиров |

### 4.2 Примеры конфигурации

#### 4.2.1 Development (без TLS)

```bash
# .env.development
TOPGUN_PORT=8080
TOPGUN_CLUSTER_PORT=9080
NODE_ID=dev-node-1
# TLS отключен по умолчанию
```

#### 4.2.2 Production (TLS для клиентов)

```bash
# .env.production
TOPGUN_PORT=443
TOPGUN_CLUSTER_PORT=9443
NODE_ID=prod-node-1

# Client TLS
TOPGUN_TLS_ENABLED=true
TOPGUN_TLS_CERT_PATH=/etc/topgun/tls/server.crt
TOPGUN_TLS_KEY_PATH=/etc/topgun/tls/server.key
TOPGUN_TLS_CA_PATH=/etc/topgun/tls/ca.crt
TOPGUN_TLS_MIN_VERSION=TLSv1.3
```

#### 4.2.3 Production (Full mTLS)

```bash
# .env.production-mtls
TOPGUN_PORT=443
TOPGUN_CLUSTER_PORT=9443
NODE_ID=prod-node-1

# Client TLS
TOPGUN_TLS_ENABLED=true
TOPGUN_TLS_CERT_PATH=/etc/topgun/tls/server.crt
TOPGUN_TLS_KEY_PATH=/etc/topgun/tls/server.key
TOPGUN_TLS_CA_PATH=/etc/topgun/tls/ca.crt
TOPGUN_TLS_MIN_VERSION=TLSv1.3

# Cluster mTLS
TOPGUN_CLUSTER_TLS_ENABLED=true
TOPGUN_CLUSTER_TLS_CERT_PATH=/etc/topgun/tls/cluster.crt
TOPGUN_CLUSTER_TLS_KEY_PATH=/etc/topgun/tls/cluster.key
TOPGUN_CLUSTER_TLS_CA_PATH=/etc/topgun/tls/cluster-ca.crt
TOPGUN_CLUSTER_MTLS=true
```

---

## 5. Docker и Kubernetes

### 5.1 Dockerfile обновления

```dockerfile
# Dockerfile
FROM node:20-alpine

WORKDIR /app

# Copy built application
COPY dist/ ./dist/
COPY package*.json ./

RUN npm ci --only=production

# Create directory for TLS certificates
RUN mkdir -p /etc/topgun/tls

# Default port (can be overridden)
EXPOSE 8080 9080

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:8080/ || exit 1

CMD ["node", "dist/start-server.js"]
```

### 5.2 Kubernetes Deployment

```yaml
# k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: topgun
spec:
  replicas: 3
  selector:
    matchLabels:
      app: topgun
  template:
    metadata:
      labels:
        app: topgun
    spec:
      containers:
      - name: topgun
        image: topgun:latest
        ports:
        - containerPort: 443
          name: https
        - containerPort: 9443
          name: cluster
        env:
        - name: TOPGUN_PORT
          value: "443"
        - name: TOPGUN_CLUSTER_PORT
          value: "9443"
        - name: TOPGUN_TLS_ENABLED
          value: "true"
        - name: TOPGUN_TLS_CERT_PATH
          value: "/etc/topgun/tls/tls.crt"
        - name: TOPGUN_TLS_KEY_PATH
          value: "/etc/topgun/tls/tls.key"
        - name: TOPGUN_CLUSTER_TLS_ENABLED
          value: "true"
        - name: TOPGUN_CLUSTER_MTLS
          value: "true"
        volumeMounts:
        - name: tls-certs
          mountPath: /etc/topgun/tls
          readOnly: true
      volumes:
      - name: tls-certs
        secret:
          secretName: topgun-tls
---
apiVersion: v1
kind: Secret
metadata:
  name: topgun-tls
type: kubernetes.io/tls
data:
  tls.crt: <base64-encoded-cert>
  tls.key: <base64-encoded-key>
  ca.crt: <base64-encoded-ca>
```

### 5.3 cert-manager интеграция

```yaml
# k8s/certificate.yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: topgun-tls
spec:
  secretName: topgun-tls
  issuerRef:
    name: letsencrypt-prod
    kind: ClusterIssuer
  dnsNames:
  - topgun.example.com
  - "*.topgun-cluster.svc.cluster.local"
```

---

## 6. Клиентские изменения

### 6.1 TopGunClient обновления

```typescript
// packages/client/src/TopGunClient.ts

export interface TopGunClientConfig {
  serverUrl: string;  // Теперь поддерживает wss://
  token?: string;
  // ... existing options ...
}

// Клиент автоматически определяет протокол из URL
// wss://server.example.com → TLS
// ws://localhost:8080 → Plain (development)
```

### 6.2 Примеры использования

```typescript
// Production
const client = new TopGunClient({
  serverUrl: 'wss://topgun.example.com',
  token: 'eyJhbGciOiJIUzI1NiIs...'
});

// Development
const client = new TopGunClient({
  serverUrl: 'ws://localhost:8080',
  token: 'dev-token'
});
```

---

## 7. Тестирование

### 7.1 Unit Tests

```typescript
// packages/server/src/__tests__/tls.test.ts

describe('TLS Configuration', () => {
  it('should create HTTPS server when TLS is enabled', async () => {
    const server = new ServerCoordinator({
      port: 0,
      nodeId: 'test-node',
      tls: {
        enabled: true,
        certPath: './test/fixtures/server.crt',
        keyPath: './test/fixtures/server.key',
      }
    });

    await server.ready();

    // Verify server is listening on HTTPS
    const response = await fetch(`https://localhost:${server.port}`, {
      agent: new https.Agent({ rejectUnauthorized: false })
    });

    expect(response.ok).toBe(true);

    await server.shutdown();
  });

  it('should reject connections with invalid certificates when mTLS is enabled', async () => {
    // ... mTLS test ...
  });

  it('should warn in production when TLS is disabled', () => {
    const originalEnv = process.env.NODE_ENV;
    process.env.NODE_ENV = 'production';

    const logSpy = jest.spyOn(logger, 'warn');

    new ServerCoordinator({
      port: 0,
      nodeId: 'test-node',
      // No TLS config
    });

    expect(logSpy).toHaveBeenCalledWith(
      expect.stringContaining('TLS is disabled')
    );

    process.env.NODE_ENV = originalEnv;
  });
});
```

### 7.2 Integration Tests

```typescript
// packages/server/src/__tests__/tls-integration.test.ts

describe('TLS Integration', () => {
  let server1: ServerCoordinator;
  let server2: ServerCoordinator;

  beforeAll(async () => {
    // Start two nodes with mTLS
    server1 = new ServerCoordinator({
      port: 0,
      clusterPort: 0,
      nodeId: 'node-1',
      tls: { enabled: true, certPath: '...', keyPath: '...' },
      clusterTls: {
        enabled: true,
        certPath: '...',
        keyPath: '...',
        caCertPath: '...',
        requireClientCert: true
      }
    });

    await server1.ready();

    server2 = new ServerCoordinator({
      port: 0,
      clusterPort: 0,
      nodeId: 'node-2',
      peers: [`localhost:${server1.clusterPort}`],
      tls: { enabled: true, certPath: '...', keyPath: '...' },
      clusterTls: {
        enabled: true,
        certPath: '...',
        keyPath: '...',
        caCertPath: '...',
        requireClientCert: true
      }
    });

    await server2.ready();
  });

  it('should establish secure cluster connection', async () => {
    // Wait for cluster formation
    await new Promise(r => setTimeout(r, 2000));

    // Verify both nodes see each other
    // ... assertions ...
  });

  afterAll(async () => {
    await server1.shutdown();
    await server2.shutdown();
  });
});
```

### 7.3 Генерация тестовых сертификатов

```bash
#!/bin/bash
# scripts/generate-test-certs.sh

# Create CA
openssl genrsa -out test/fixtures/ca.key 4096
openssl req -new -x509 -days 365 -key test/fixtures/ca.key \
  -out test/fixtures/ca.crt -subj "/CN=TopGun Test CA"

# Create Server Certificate
openssl genrsa -out test/fixtures/server.key 2048
openssl req -new -key test/fixtures/server.key \
  -out test/fixtures/server.csr -subj "/CN=localhost"
openssl x509 -req -days 365 -in test/fixtures/server.csr \
  -CA test/fixtures/ca.crt -CAkey test/fixtures/ca.key \
  -CAcreateserial -out test/fixtures/server.crt \
  -extfile <(echo "subjectAltName=DNS:localhost,IP:127.0.0.1")

# Create Cluster Node Certificates
for i in 1 2 3; do
  openssl genrsa -out test/fixtures/node${i}.key 2048
  openssl req -new -key test/fixtures/node${i}.key \
    -out test/fixtures/node${i}.csr -subj "/CN=node-${i}"
  openssl x509 -req -days 365 -in test/fixtures/node${i}.csr \
    -CA test/fixtures/ca.crt -CAkey test/fixtures/ca.key \
    -CAcreateserial -out test/fixtures/node${i}.crt
done

echo "Test certificates generated in test/fixtures/"
```

---

## 8. Миграция

### 8.1 План миграции

#### Этап 1: Подготовка (без downtime)
1. Обновить код сервера с поддержкой TLS (но отключенным по умолчанию)
2. Задеплоить обновление
3. Подготовить сертификаты
4. Обновить клиентов для поддержки `wss://`

#### Этап 2: Включение TLS (rolling update)
1. Настроить Load Balancer для TLS termination (опционально)
2. Включить TLS на одной ноде, проверить
3. Rolling update остальных нод
4. Переключить клиентов на `wss://`

#### Этап 3: Cluster mTLS (maintenance window)
1. Сгенерировать cluster сертификаты для всех нод
2. Координированный рестарт кластера с mTLS
3. Верифицировать cluster connectivity

### 8.2 Rollback план

```bash
# В случае проблем с TLS:
# 1. Отключить TLS через env var
TOPGUN_TLS_ENABLED=false

# 2. Рестарт сервисов
kubectl rollout restart deployment/topgun

# 3. Переключить клиентов обратно на ws://
```

---

## 9. Мониторинг и алерты

### 9.1 Новые метрики

```typescript
// packages/server/src/monitoring/MetricsService.ts

// Добавить метрики:
const tlsConnectionsTotal = new Counter({
  name: 'topgun_tls_connections_total',
  help: 'Total TLS connections',
  labelNames: ['protocol_version', 'cipher']
});

const tlsHandshakeErrors = new Counter({
  name: 'topgun_tls_handshake_errors_total',
  help: 'TLS handshake errors',
  labelNames: ['error_type']
});

const certificateExpiryDays = new Gauge({
  name: 'topgun_certificate_expiry_days',
  help: 'Days until certificate expiry',
  labelNames: ['cert_type'] // 'server', 'cluster'
});
```

### 9.2 Prometheus Alerts

```yaml
# prometheus/alerts.yaml
groups:
- name: topgun-tls
  rules:
  - alert: TLSCertificateExpiringSoon
    expr: topgun_certificate_expiry_days < 30
    for: 1h
    labels:
      severity: warning
    annotations:
      summary: "TLS certificate expiring soon"
      description: "Certificate {{ $labels.cert_type }} expires in {{ $value }} days"

  - alert: TLSCertificateExpired
    expr: topgun_certificate_expiry_days < 0
    for: 0m
    labels:
      severity: critical
    annotations:
      summary: "TLS certificate has expired!"

  - alert: HighTLSHandshakeErrors
    expr: rate(topgun_tls_handshake_errors_total[5m]) > 10
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "High rate of TLS handshake errors"
```

---

## 10. Безопасность

### 10.1 Рекомендуемые Cipher Suites

```typescript
// Для TLS 1.3 (рекомендуется)
const TLS13_CIPHERS = [
  'TLS_AES_256_GCM_SHA384',
  'TLS_CHACHA20_POLY1305_SHA256',
  'TLS_AES_128_GCM_SHA256'
].join(':');

// Для TLS 1.2 (если требуется совместимость)
const TLS12_CIPHERS = [
  'ECDHE-ECDSA-AES256-GCM-SHA384',
  'ECDHE-RSA-AES256-GCM-SHA384',
  'ECDHE-ECDSA-CHACHA20-POLY1305',
  'ECDHE-RSA-CHACHA20-POLY1305',
  'ECDHE-ECDSA-AES128-GCM-SHA256',
  'ECDHE-RSA-AES128-GCM-SHA256'
].join(':');
```

### 10.2 Рекомендации по ключам

| Тип ключа | Минимальный размер | Рекомендуемый размер |
|-----------|-------------------|---------------------|
| RSA | 2048 bits | 4096 bits |
| ECDSA | P-256 | P-384 |
| Ed25519 | N/A | Предпочтительный |

### 10.3 Certificate Pinning (опционально)

```typescript
// Для критичных deployments можно добавить certificate pinning
interface TLSConfig {
  // ... existing fields ...

  /**
   * SHA-256 fingerprints разрешённых сертификатов
   * Используется для certificate pinning
   */
  pinnedCertificates?: string[];
}
```

---

## 11. Checklist для релиза

### 11.1 Код
- [ ] Создать `packages/server/src/types/TLSConfig.ts`
- [ ] Обновить `ServerCoordinator.ts` с поддержкой HTTPS
- [ ] Обновить `ClusterManager.ts` с поддержкой WSS и mTLS
- [ ] Обновить `start-server.ts` с новыми env vars
- [ ] Добавить TLS метрики в `MetricsService.ts`
- [ ] Добавить warning logs для disabled TLS в production

### 11.2 Тесты
- [ ] Unit tests для TLS configuration
- [ ] Integration tests для WSS connections
- [ ] Integration tests для cluster mTLS
- [ ] Скрипт генерации тестовых сертификатов

### 11.3 Документация
- [ ] Обновить README с TLS instructions
- [ ] Добавить примеры Docker/K8s конфигурации
- [ ] Документировать migration path

### 11.4 Infrastructure
- [ ] Подготовить Prometheus alerts
- [ ] Настроить certificate monitoring
- [ ] Подготовить rollback plan

---

## 12. Приложения

### A. Полезные команды OpenSSL

```bash
# Проверить сертификат
openssl x509 -in server.crt -text -noout

# Проверить expiry date
openssl x509 -enddate -noout -in server.crt

# Проверить соответствие ключа и сертификата
openssl x509 -noout -modulus -in server.crt | openssl md5
openssl rsa -noout -modulus -in server.key | openssl md5

# Тестовое подключение
openssl s_client -connect localhost:8080 -tls1_3

# Получить certificate chain
openssl s_client -showcerts -connect server.example.com:443
```

### B. Troubleshooting

| Проблема | Возможная причина | Решение |
|----------|------------------|---------|
| `UNABLE_TO_VERIFY_LEAF_SIGNATURE` | Отсутствует CA cert | Добавить `caCertPath` |
| `CERT_HAS_EXPIRED` | Сертификат истёк | Обновить сертификат |
| `DEPTH_ZERO_SELF_SIGNED_CERT` | Self-signed в production | Использовать CA-signed cert или `rejectUnauthorized: false` (dev only) |
| `ERR_TLS_CERT_ALTNAME_INVALID` | Hostname не соответствует CN/SAN | Добавить правильный SAN в сертификат |

---

## 13. Ссылки

- [Node.js TLS Documentation](https://nodejs.org/api/tls.html)
- [ws Library TLS Options](https://github.com/websockets/ws/blob/master/doc/ws.md)
- [Mozilla SSL Configuration Generator](https://ssl-config.mozilla.org/)
- [Let's Encrypt](https://letsencrypt.org/)
- [cert-manager](https://cert-manager.io/)
