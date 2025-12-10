import { ServerCoordinator } from '../ServerCoordinator';
import * as path from 'path';

describe('TLS Integration', () => {
    const FIXTURES_DIR = path.resolve(__dirname, '../../test/fixtures');
    const CA_PATH = path.join(FIXTURES_DIR, 'ca.crt');

    // Node 1 Certs
    const NODE1_CERT = path.join(FIXTURES_DIR, 'node1.crt');
    const NODE1_KEY = path.join(FIXTURES_DIR, 'node1.key');

    // Node 2 Certs
    const NODE2_CERT = path.join(FIXTURES_DIR, 'node2.crt');
    const NODE2_KEY = path.join(FIXTURES_DIR, 'node2.key');

    // Node 3 Certs (for invalid cert test)
    const NODE3_CERT = path.join(FIXTURES_DIR, 'node3.crt');
    const NODE3_KEY = path.join(FIXTURES_DIR, 'node3.key');

    describe('Valid mTLS Connection', () => {
        let server1: ServerCoordinator;
        let server2: ServerCoordinator;

        beforeAll(async () => {
            // Start Node 1
            server1 = new ServerCoordinator({
                port: 0,
                clusterPort: 0,
                metricsPort: 0,
                nodeId: 'node-1',
                tls: {
                    enabled: true,
                    certPath: NODE1_CERT,
                    keyPath: NODE1_KEY,
                },
                clusterTls: {
                    enabled: true,
                    certPath: NODE1_CERT,
                    keyPath: NODE1_KEY,
                    caCertPath: CA_PATH,
                    requireClientCert: true,
                    rejectUnauthorized: true
                }
            });
            await server1.ready();

            // Start Node 2 (connects to Node 1)
            server2 = new ServerCoordinator({
                port: 0,
                clusterPort: 0,
                metricsPort: 0,
                nodeId: 'node-2',
                peers: [`localhost:${server1.clusterPort}`],
                tls: {
                    enabled: true,
                    certPath: NODE2_CERT,
                    keyPath: NODE2_KEY,
                },
                clusterTls: {
                    enabled: true,
                    certPath: NODE2_CERT,
                    keyPath: NODE2_KEY,
                    caCertPath: CA_PATH,
                    requireClientCert: true,
                    rejectUnauthorized: true
                }
            });
            await server2.ready();
        });

        afterAll(async () => {
            if (server1) await server1.shutdown();
            if (server2) await server2.shutdown();
        });

        it('should establish secure cluster connection', async () => {
            // Wait for cluster formation (backoff might trigger)
            await new Promise(r => setTimeout(r, 2000));

            const cluster1 = (server1 as any).cluster;
            const members1 = cluster1.getMembers();

            const cluster2 = (server2 as any).cluster;
            const members2 = cluster2.getMembers();

            expect(members1).toContain('node-2');
            expect(members2).toContain('node-1');
        });
    });

    describe('Invalid Certificate Rejection', () => {
        let serverWithMTLS: ServerCoordinator;
        let serverWithoutClientCert: ServerCoordinator;

        beforeAll(async () => {
            // Start server requiring mTLS
            serverWithMTLS = new ServerCoordinator({
                port: 0,
                clusterPort: 0,
                metricsPort: 0,
                nodeId: 'mtls-server',
                tls: {
                    enabled: true,
                    certPath: NODE1_CERT,
                    keyPath: NODE1_KEY,
                },
                clusterTls: {
                    enabled: true,
                    certPath: NODE1_CERT,
                    keyPath: NODE1_KEY,
                    caCertPath: CA_PATH,
                    requireClientCert: true,
                    rejectUnauthorized: true
                }
            });
            await serverWithMTLS.ready();

            // Start server WITHOUT providing client cert for cluster (simulates invalid/missing cert)
            serverWithoutClientCert = new ServerCoordinator({
                port: 0,
                clusterPort: 0,
                metricsPort: 0,
                nodeId: 'no-client-cert',
                peers: [`localhost:${serverWithMTLS.clusterPort}`],
                tls: {
                    enabled: true,
                    certPath: NODE3_CERT,
                    keyPath: NODE3_KEY,
                },
                clusterTls: {
                    enabled: true,
                    // Intentionally NOT providing certPath/keyPath for mTLS client auth
                    // Only providing CA to verify server, but no client cert
                    certPath: NODE3_CERT,
                    keyPath: NODE3_KEY,
                    // Use a different CA so the cert won't be trusted
                    caCertPath: NODE3_CERT, // Wrong CA - should cause rejection
                    requireClientCert: false,
                    rejectUnauthorized: false // Allow connection attempt even with bad cert
                }
            });
            await serverWithoutClientCert.ready();
        });

        afterAll(async () => {
            if (serverWithMTLS) await serverWithMTLS.shutdown();
            if (serverWithoutClientCert) await serverWithoutClientCert.shutdown();
        });

        it('should NOT establish connection when certificates are mismatched', async () => {
            // Wait for connection attempts
            await new Promise(r => setTimeout(r, 3000));

            const clusterMTLS = (serverWithMTLS as any).cluster;
            const membersMTLS = clusterMTLS.getMembers();

            // The mTLS server should NOT have the invalid client in its members
            // It should only have itself
            expect(membersMTLS).not.toContain('no-client-cert');
            expect(membersMTLS).toHaveLength(1); // Only self
            expect(membersMTLS).toContain('mtls-server');
        });
    });
});
