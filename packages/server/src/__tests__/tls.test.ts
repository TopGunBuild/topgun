import { ServerCoordinator } from '../ServerCoordinator';
import * as https from 'https';
import * as path from 'path';
import { logger } from '../utils/logger';

// Mock logger manually
jest.mock('../utils/logger', () => ({
    logger: {
        info: jest.fn(),
        warn: jest.fn(),
        error: jest.fn(),
    }
}));

describe('TLS Configuration', () => {
    const FIXTURES_DIR = path.resolve(__dirname, '../../test/fixtures');
    const CERT_PATH = path.join(FIXTURES_DIR, 'server.crt');
    const KEY_PATH = path.join(FIXTURES_DIR, 'server.key');

    let server: ServerCoordinator;

    afterEach(async () => {
        if (server) {
            await server.shutdown();
        }
    });

    it('should create HTTPS server when TLS is enabled', async () => {
        server = new ServerCoordinator({
            port: 0,
            metricsPort: 0, // Random port to avoid conflict
            nodeId: 'test-node-tls',
            tls: {
                enabled: true,
                certPath: CERT_PATH,
                keyPath: KEY_PATH,
            }
        });

        await server.ready();

        // Verify server is listening
        expect(server.port).toBeGreaterThan(0);

        // Try to connect via HTTPS (should fail with self-signed cert if rejected)
        const agent = new https.Agent({ rejectUnauthorized: false });

        const responsePromise = new Promise<{ statusCode: number }>((resolve, reject) => {
            const req = https.get(`https://localhost:${server.port}`, { agent }, (res) => {
                resolve({ statusCode: res.statusCode || 0 });
            });
            req.on('error', reject);
        });

        const response = await responsePromise;
        expect(response.statusCode).toBe(200);
    });

    it('should warn in production when TLS is disabled', () => {
        const originalEnv = process.env.NODE_ENV;
        process.env.NODE_ENV = 'production';

        // Spy is already effective due to manual mock, but let's just check call count or args
        // Since we mocked it with jest.fn(), we can import it and check.

        server = new ServerCoordinator({
            port: 0,
            metricsPort: 0,
            nodeId: 'test-node-no-tls',
            // No TLS config
        });

        expect(logger.warn).toHaveBeenCalledWith(
            expect.stringContaining('TLS is disabled')
        );

        process.env.NODE_ENV = originalEnv;
    });
});
