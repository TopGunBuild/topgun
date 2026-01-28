import { ServerFactory } from './ServerFactory';
import { PostgresAdapter } from './storage/PostgresAdapter';
import { logger } from './utils/logger';
import { TLSConfig, ClusterTLSConfig } from './types/TLSConfig';
import { createBootstrapController } from './bootstrap';

// Configuration
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

// Main startup function (async to support auto-setup)
async function main() {
    // Phase 14D-2: Zero-Touch Setup
    // Run auto-setup before creating ServerCoordinator
    const bootstrapController = createBootstrapController();
    await bootstrapController.checkAutoSetup();

    // Setup Storage
    let storage;
    if (DB_URL) {
        storage = new PostgresAdapter({ connectionString: DB_URL });
        logger.info('Using PostgresAdapter with DATABASE_URL');
    } else {
        logger.info('No DATABASE_URL provided, using in-memory storage (non-persistent)');
    }

    const server = ServerFactory.create({
        port: PORT,
        clusterPort: CLUSTER_PORT,
        nodeId: NODE_ID,
        peers: PEERS,
        discovery: DISCOVERY_MODE,
        serviceName: DISCOVERY_SERVICE,
        discoveryInterval: DISCOVERY_INTERVAL,
        storage,
        host: '0.0.0.0', // Bind to all interfaces in Docker
        securityPolicies: [
            // Default permissive policy for now - in prod this should be configured
            {
                role: 'USER',
                mapNamePattern: '*',
                actions: ['ALL']
            }
        ],
        tls: tlsConfig,
        clusterTls: clusterTlsConfig,
    });

    // Graceful Shutdown
    const shutdown = async (signal: string) => {
        logger.info({ signal }, 'Starting graceful shutdown');
        try {
            await server.shutdown();
            process.exit(0);
        } catch (err) {
            logger.error({ err }, 'Error during shutdown');
            process.exit(1);
        }
    };

    process.on('SIGTERM', () => shutdown('SIGTERM'));
    process.on('SIGINT', () => shutdown('SIGINT'));

    logger.info({ port: PORT, clusterPort: CLUSTER_PORT, nodeId: NODE_ID, discovery: DISCOVERY_MODE }, 'TopGun Server Starting');
}

// Run main and handle errors
main().catch((err) => {
    logger.error({ err }, 'Failed to start server');
    process.exit(1);
});

