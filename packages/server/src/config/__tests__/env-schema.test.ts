import { validateEnv, EnvConfig } from '../env-schema';

describe('validateEnv', () => {
    let originalEnv: NodeJS.ProcessEnv;
    let consoleErrorSpy: jest.SpyInstance;
    let exitSpy: jest.SpyInstance;

    beforeEach(() => {
        originalEnv = { ...process.env };
        consoleErrorSpy = jest.spyOn(console, 'error').mockImplementation();
        exitSpy = jest.spyOn(process, 'exit').mockImplementation(() => {
            throw new Error('process.exit called');
        });
    });

    afterEach(() => {
        process.env = originalEnv;
        consoleErrorSpy.mockRestore();
        exitSpy.mockRestore();
    });

    describe('Default values', () => {
        it('should apply default values when env vars not set', () => {
            process.env = {};
            const config = validateEnv();

            expect(config.NODE_ENV).toBe('development');
            expect(config.TOPGUN_PORT).toBe(8080);
            expect(config.TOPGUN_CLUSTER_PORT).toBe(9080);
            expect(config.TOPGUN_DISCOVERY_INTERVAL).toBe(10000);
            expect(config.TOPGUN_TLS_ENABLED).toBe(false);
            expect(config.TOPGUN_CLUSTER_TLS_ENABLED).toBe(false);
            expect(config.TOPGUN_TLS_MIN_VERSION).toBe('TLSv1.2');
            expect(config.TOPGUN_DEBUG).toBe(false);
            expect(config.TOPGUN_CLUSTER_MTLS).toBe(false);
            expect(config.TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED).toBe(true);
        });

        it('should use provided values over defaults', () => {
            process.env = {
                NODE_ENV: 'production',
                TOPGUN_PORT: '3000',
                TOPGUN_CLUSTER_PORT: '4000',
                TOPGUN_DISCOVERY_INTERVAL: '5000',
                TOPGUN_DEBUG: 'true',
                JWT_SECRET: 'a'.repeat(32),
            };
            const config = validateEnv();

            expect(config.NODE_ENV).toBe('production');
            expect(config.TOPGUN_PORT).toBe(3000);
            expect(config.TOPGUN_CLUSTER_PORT).toBe(4000);
            expect(config.TOPGUN_DISCOVERY_INTERVAL).toBe(5000);
            expect(config.TOPGUN_DEBUG).toBe(true);
        });
    });

    describe('Type coercion', () => {
        it('should coerce string port numbers to integers', () => {
            process.env = {
                TOPGUN_PORT: '9999',
                TOPGUN_CLUSTER_PORT: '8888',
                TOPGUN_METRICS_PORT: '7777',
            };
            const config = validateEnv();

            expect(config.TOPGUN_PORT).toBe(9999);
            expect(typeof config.TOPGUN_PORT).toBe('number');
            expect(config.TOPGUN_CLUSTER_PORT).toBe(8888);
            expect(typeof config.TOPGUN_CLUSTER_PORT).toBe('number');
            expect(config.TOPGUN_METRICS_PORT).toBe(7777);
            expect(typeof config.TOPGUN_METRICS_PORT).toBe('number');
        });

        it('should transform string "true"/"false" to boolean', () => {
            process.env = {
                TOPGUN_DEBUG: 'true',
                TOPGUN_CLUSTER_MTLS: 'false',
                TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED: 'false',
            };
            const config = validateEnv();

            expect(config.TOPGUN_TLS_ENABLED).toBe(false);
            expect(typeof config.TOPGUN_TLS_ENABLED).toBe('boolean');
            expect(config.TOPGUN_CLUSTER_TLS_ENABLED).toBe(false);
            expect(typeof config.TOPGUN_CLUSTER_TLS_ENABLED).toBe('boolean');
            expect(config.TOPGUN_DEBUG).toBe(true);
            expect(typeof config.TOPGUN_DEBUG).toBe('boolean');
            expect(config.TOPGUN_CLUSTER_MTLS).toBe(false);
            expect(typeof config.TOPGUN_CLUSTER_MTLS).toBe('boolean');
            expect(config.TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED).toBe(false);
            expect(typeof config.TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED).toBe('boolean');
        });
    });

    describe('Port validation', () => {
        it('should reject negative port numbers', () => {
            process.env = {
                TOPGUN_PORT: '-1',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('TOPGUN_PORT'),
            );
            expect(exitSpy).toHaveBeenCalledWith(1);
        });

        it('should reject zero port number', () => {
            process.env = {
                TOPGUN_PORT: '0',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('TOPGUN_PORT'),
            );
        });

        it('should reject port numbers greater than 65535', () => {
            process.env = {
                TOPGUN_CLUSTER_PORT: '99999',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('TOPGUN_CLUSTER_PORT'),
            );
        });

        it('should accept valid port numbers', () => {
            process.env = {
                TOPGUN_PORT: '1',
                TOPGUN_CLUSTER_PORT: '65535',
                TOPGUN_METRICS_PORT: '8888',
            };

            const config = validateEnv();
            expect(config.TOPGUN_PORT).toBe(1);
            expect(config.TOPGUN_CLUSTER_PORT).toBe(65535);
            expect(config.TOPGUN_METRICS_PORT).toBe(8888);
        });

        it('should reject non-numeric port values', () => {
            process.env = {
                TOPGUN_PORT: 'abc',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('TOPGUN_PORT'),
            );
        });
    });

    describe('Production mode requirements', () => {
        it('should require JWT_SECRET in production', () => {
            process.env = {
                NODE_ENV: 'production',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('JWT_SECRET is required in production'),
            );
            expect(exitSpy).toHaveBeenCalledWith(1);
        });

        it('should accept JWT_SECRET with at least 32 characters in production', () => {
            process.env = {
                NODE_ENV: 'production',
                JWT_SECRET: 'a'.repeat(32),
            };

            const config = validateEnv();
            expect(config.NODE_ENV).toBe('production');
            expect(config.JWT_SECRET).toBe('a'.repeat(32));
        });

        it('should reject JWT_SECRET with less than 32 characters', () => {
            process.env = {
                NODE_ENV: 'production',
                JWT_SECRET: 'tooshort',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('JWT_SECRET'),
            );
        });

        it('should not require JWT_SECRET in development', () => {
            process.env = {
                NODE_ENV: 'development',
            };

            const config = validateEnv();
            expect(config.NODE_ENV).toBe('development');
            expect(config.JWT_SECRET).toBeUndefined();
        });

        it('should not require JWT_SECRET in test mode', () => {
            process.env = {
                NODE_ENV: 'test',
            };

            const config = validateEnv();
            expect(config.NODE_ENV).toBe('test');
            expect(config.JWT_SECRET).toBeUndefined();
        });
    });

    describe('TLS validation', () => {
        it('should require cert and key paths when TLS enabled', () => {
            process.env = {
                TOPGUN_TLS_ENABLED: 'true',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            const errorMessage = consoleErrorSpy.mock.calls[0][0];
            expect(errorMessage).toContain('TOPGUN_TLS_CERT_PATH');
            expect(errorMessage).toContain('TOPGUN_TLS_KEY_PATH');
            expect(exitSpy).toHaveBeenCalledWith(1);
        });

        it('should accept TLS config with cert and key paths', () => {
            process.env = {
                TOPGUN_TLS_ENABLED: 'true',
                TOPGUN_TLS_CERT_PATH: '/path/to/cert.pem',
                TOPGUN_TLS_KEY_PATH: '/path/to/key.pem',
            };

            const config = validateEnv();
            expect(config.TOPGUN_TLS_ENABLED).toBe(true);
            expect(config.TOPGUN_TLS_CERT_PATH).toBe('/path/to/cert.pem');
            expect(config.TOPGUN_TLS_KEY_PATH).toBe('/path/to/key.pem');
        });

        it('should accept optional TLS CA path and passphrase', () => {
            process.env = {
                TOPGUN_TLS_ENABLED: 'true',
                TOPGUN_TLS_CERT_PATH: '/path/to/cert.pem',
                TOPGUN_TLS_KEY_PATH: '/path/to/key.pem',
                TOPGUN_TLS_CA_PATH: '/path/to/ca.pem',
                TOPGUN_TLS_PASSPHRASE: 'secret',
                TOPGUN_TLS_MIN_VERSION: 'TLSv1.3',
            };

            const config = validateEnv();
            expect(config.TOPGUN_TLS_CA_PATH).toBe('/path/to/ca.pem');
            expect(config.TOPGUN_TLS_PASSPHRASE).toBe('secret');
            expect(config.TOPGUN_TLS_MIN_VERSION).toBe('TLSv1.3');
        });
    });

    describe('Cluster TLS validation', () => {
        it('should require cert/key paths when cluster TLS enabled', () => {
            process.env = {
                TOPGUN_CLUSTER_TLS_ENABLED: 'true',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            const errorMessage = consoleErrorSpy.mock.calls[0][0];
            expect(errorMessage).toContain('Cluster TLS requires cert path');
            expect(errorMessage).toContain('Cluster TLS requires key path');
            expect(exitSpy).toHaveBeenCalledWith(1);
        });

        it('should accept cluster-specific TLS paths', () => {
            process.env = {
                TOPGUN_CLUSTER_TLS_ENABLED: 'true',
                TOPGUN_CLUSTER_TLS_CERT_PATH: '/path/to/cluster-cert.pem',
                TOPGUN_CLUSTER_TLS_KEY_PATH: '/path/to/cluster-key.pem',
            };

            const config = validateEnv();
            expect(config.TOPGUN_CLUSTER_TLS_ENABLED).toBe(true);
            expect(config.TOPGUN_CLUSTER_TLS_CERT_PATH).toBe(
                '/path/to/cluster-cert.pem',
            );
            expect(config.TOPGUN_CLUSTER_TLS_KEY_PATH).toBe(
                '/path/to/cluster-key.pem',
            );
        });

        it('should allow cluster TLS to fallback to main TLS paths', () => {
            process.env = {
                TOPGUN_CLUSTER_TLS_ENABLED: 'true',
                TOPGUN_TLS_CERT_PATH: '/path/to/cert.pem',
                TOPGUN_TLS_KEY_PATH: '/path/to/key.pem',
            };

            const config = validateEnv();
            expect(config.TOPGUN_CLUSTER_TLS_ENABLED).toBe(true);
            expect(config.TOPGUN_TLS_CERT_PATH).toBe('/path/to/cert.pem');
            expect(config.TOPGUN_TLS_KEY_PATH).toBe('/path/to/key.pem');
        });

        it('should accept cluster mTLS and reject unauthorized settings', () => {
            process.env = {
                TOPGUN_CLUSTER_TLS_ENABLED: 'true',
                TOPGUN_CLUSTER_TLS_CERT_PATH: '/path/to/cert.pem',
                TOPGUN_CLUSTER_TLS_KEY_PATH: '/path/to/key.pem',
                TOPGUN_CLUSTER_MTLS: 'true',
                TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED: 'false',
            };

            const config = validateEnv();
            expect(config.TOPGUN_CLUSTER_MTLS).toBe(true);
            expect(config.TOPGUN_CLUSTER_TLS_REJECT_UNAUTHORIZED).toBe(false);
        });
    });

    describe('Error message collection', () => {
        it('should collect and report all validation errors', () => {
            process.env = {
                NODE_ENV: 'production',
                TOPGUN_PORT: '-1',
                TOPGUN_CLUSTER_PORT: '99999',
                TOPGUN_TLS_ENABLED: 'true',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            const errorMessage = consoleErrorSpy.mock.calls[0][0];

            expect(errorMessage).toContain('Environment validation failed');
            expect(errorMessage).toContain('TOPGUN_PORT');
            expect(errorMessage).toContain('TOPGUN_CLUSTER_PORT');
            expect(errorMessage).toContain('JWT_SECRET');
            expect(errorMessage).toContain('TOPGUN_TLS_CERT_PATH');
            expect(errorMessage).toContain('TOPGUN_TLS_KEY_PATH');
        });
    });

    describe('Optional fields', () => {
        it('should accept undefined optional fields', () => {
            process.env = {};

            const config = validateEnv();
            expect(config.NODE_ID).toBeUndefined();
            expect(config.TOPGUN_PEERS).toBeUndefined();
            expect(config.TOPGUN_DISCOVERY_SERVICE).toBeUndefined();
            expect(config.DATABASE_URL).toBeUndefined();
            expect(config.JWT_SECRET).toBeUndefined();
            expect(config.TOPGUN_METRICS_PORT).toBeUndefined();
            expect(config.TOPGUN_TLS_CERT_PATH).toBeUndefined();
            expect(config.TOPGUN_TLS_CA_PATH).toBeUndefined();
        });

        it('should accept provided optional values', () => {
            process.env = {
                NODE_ID: 'custom-node-123',
                TOPGUN_PEERS: 'peer1,peer2',
                TOPGUN_DISCOVERY_SERVICE: 'kubernetes.default.svc',
                DATABASE_URL: 'postgres://localhost/mydb',
                TOPGUN_METRICS_PORT: '9090',
            };

            const config = validateEnv();
            expect(config.NODE_ID).toBe('custom-node-123');
            expect(config.TOPGUN_PEERS).toBe('peer1,peer2');
            expect(config.TOPGUN_DISCOVERY_SERVICE).toBe(
                'kubernetes.default.svc',
            );
            expect(config.DATABASE_URL).toBe('postgres://localhost/mydb');
            expect(config.TOPGUN_METRICS_PORT).toBe(9090);
        });
    });

    describe('NODE_ENV validation', () => {
        it('should only accept valid NODE_ENV values', () => {
            process.env = {
                NODE_ENV: 'invalid',
            };

            expect(() => validateEnv()).toThrow('process.exit called');
            expect(consoleErrorSpy).toHaveBeenCalledWith(
                expect.stringContaining('NODE_ENV'),
            );
        });

        it('should accept valid NODE_ENV values', () => {
            const validEnvs: Array<'development' | 'test' | 'production'> = [
                'development',
                'test',
                'production',
            ];

            for (const env of validEnvs) {
                process.env = {
                    NODE_ENV: env,
                    // Production requires JWT_SECRET
                    ...(env === 'production' ? { JWT_SECRET: 'a'.repeat(32) } : {}),
                };
                const config = validateEnv();
                expect(config.NODE_ENV).toBe(env);
            }
        });
    });

    describe('Type inference', () => {
        it('should export correct TypeScript type', () => {
            process.env = {
                TOPGUN_PORT: '8080',
            };
            const config = validateEnv();

            // Type assertions to verify TypeScript inference
            const port: number = config.TOPGUN_PORT;
            const debug: boolean = config.TOPGUN_DEBUG;
            const nodeEnv: 'development' | 'test' | 'production' =
                config.NODE_ENV;
            const nodeId: string | undefined = config.NODE_ID;
            const tlsMinVersion: 'TLSv1.2' | 'TLSv1.3' =
                config.TOPGUN_TLS_MIN_VERSION;

            expect(port).toBe(8080);
            expect(debug).toBe(false);
            expect(nodeEnv).toBe('development');
            expect(nodeId).toBeUndefined();
            expect(tlsMinVersion).toBe('TLSv1.2');
        });
    });
});
