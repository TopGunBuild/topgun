import { ServerFactory } from './ServerFactory';
import { PostgresAdapter } from './storage/PostgresAdapter';
import { logger } from './utils/logger';
import { TLSConfig, ClusterTLSConfig } from './types/TLSConfig';
import { createBootstrapController } from './bootstrap';
import { validateEnv } from './config';

// Validate environment variables
const env = validateEnv();

// Configuration
const PORT = env.TOPGUN_PORT;
const CLUSTER_PORT = env.TOPGUN_CLUSTER_PORT;
const NODE_ID = env.NODE_ID || `node-${Math.random().toString(36).substring(2, 8)}`;
const PEERS = env.TOPGUN_PEERS ? env.TOPGUN_PEERS.split(',') : [];
const DISCOVERY_SERVICE = env.TOPGUN_DISCOVERY_SERVICE;
const DISCOVERY_INTERVAL = env.TOPGUN_DISCOVERY_INTERVAL;
const DISCOVERY_MODE = DISCOVERY_SERVICE ? 'kubernetes' : 'manual';

const DB_URL = env.DATABASE_URL;

// TLS Configuration
const TLS_ENABLED = env.TOPGUN_TLS_ENABLED;
const TLS_CERT_PATH = env.TOPGUN_TLS_CERT_PATH;
const TLS_KEY_PATH = env.TOPGUN_TLS_KEY_PATH;
const TLS_CA_PATH = env.TOPGUN_TLS_CA_PATH;
const TLS_MIN_VERSION = env.TOPGUN_TLS_MIN_VERSION;
const TLS_PASSPHRASE = env.TOPGUN_TLS_PASSPHRASE;

// Cluster TLS (can use same certs or separate)
const CLUSTER_TLS_ENABLED = env.TOPGUN_CLUSTER_TLS_ENABLED;
const CLUSTER_TLS_CERT_PATH = env.TOPGUN_CLUSTER_TLS_CERT_PATH || TLS_CERT_PATH;
const CLUSTER_TLS_KEY_PATH = env.TOPGUN_CLUSTER_TLS_KEY_PATH || TLS_KEY_PATH;
const CLUSTER_TLS_CA_PATH = env.TOPGUN_CLUSTER_TLS_CA_PATH || TLS_CA_PATH;
const CLUSTER_TLS_REQUIRE_CLIENT_CERT = env.TOPGUN_CLUSTER_MTLS;
const CLUSTER_TLS_REJECT_UNAUTHORIZED = env.TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED;

// Build TLS Config
let tlsConfig: TLSConfig | undefined;
if (TLS_ENABLED) {
    tlsConfig = {
        enabled: true,
        certPath: TLS_CERT_PATH!,
        keyPath: TLS_KEY_PATH!,
        caCertPath: TLS_CA_PATH,
        minVersion: TLS_MIN_VERSION,
        passphrase: TLS_PASSPHRASE,
    };

    logger.info({ certPath: TLS_CERT_PATH, minVersion: TLS_MIN_VERSION }, 'Client TLS configured');
}

// Build Cluster TLS Config
let clusterTlsConfig: ClusterTLSConfig | undefined;
if (CLUSTER_TLS_ENABLED) {
    clusterTlsConfig = {
        enabled: true,
        certPath: CLUSTER_TLS_CERT_PATH!,
        keyPath: CLUSTER_TLS_KEY_PATH!,
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
    // Zero-Touch Setup
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

