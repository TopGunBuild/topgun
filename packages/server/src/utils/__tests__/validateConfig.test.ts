import { validateJwtSecret, DEFAULT_JWT_SECRET } from '../validateConfig';

describe('validateJwtSecret', () => {
    const originalEnv = process.env.NODE_ENV;

    afterEach(() => {
        process.env.NODE_ENV = originalEnv;
    });

    describe('production mode', () => {
        beforeEach(() => {
            process.env.NODE_ENV = 'production';
        });

        test('should throw if no secret provided', () => {
            expect(() => validateJwtSecret(undefined, undefined))
                .toThrow('SECURITY ERROR: JWT_SECRET is required in production mode');
        });

        test('should throw if only default secret provided via config', () => {
            expect(() => validateJwtSecret('topgun-secret-dev', undefined))
                .toThrow('Default JWT_SECRET cannot be used in production mode');
        });

        test('should throw if only default secret provided via env', () => {
            expect(() => validateJwtSecret(undefined, 'topgun-secret-dev'))
                .toThrow('Default JWT_SECRET cannot be used in production mode');
        });

        test('should return config secret when valid', () => {
            const result = validateJwtSecret('my-secure-secret', undefined);
            expect(result).toBe('my-secure-secret');
        });

        test('should return env secret when valid and config not provided', () => {
            const result = validateJwtSecret(undefined, 'env-secure-secret');
            expect(result).toBe('env-secure-secret');
        });

        test('should prefer config secret over env secret', () => {
            const result = validateJwtSecret('config-secret', 'env-secret');
            expect(result).toBe('config-secret');
        });

        test('should include generation hint in missing secret error', () => {
            expect(() => validateJwtSecret(undefined, undefined))
                .toThrow('openssl rand -base64 32');
        });

        test('should include generation hint in default secret error', () => {
            expect(() => validateJwtSecret(DEFAULT_JWT_SECRET, undefined))
                .toThrow('openssl rand -base64 32');
        });
    });

    describe('development mode', () => {
        beforeEach(() => {
            process.env.NODE_ENV = 'development';
        });

        test('should return default secret when none provided', () => {
            const result = validateJwtSecret(undefined, undefined);
            expect(result).toBe('topgun-secret-dev');
        });

        test('should allow default secret explicitly', () => {
            const result = validateJwtSecret('topgun-secret-dev', undefined);
            expect(result).toBe('topgun-secret-dev');
        });

        test('should return provided secret when available', () => {
            const result = validateJwtSecret('custom-secret', undefined);
            expect(result).toBe('custom-secret');
        });
    });

    describe('test mode (default)', () => {
        beforeEach(() => {
            process.env.NODE_ENV = 'test';
        });

        test('should return default secret when none provided', () => {
            const result = validateJwtSecret(undefined, undefined);
            expect(result).toBe('topgun-secret-dev');
        });

        test('should allow default secret explicitly', () => {
            const result = validateJwtSecret(DEFAULT_JWT_SECRET, undefined);
            expect(result).toBe(DEFAULT_JWT_SECRET);
        });
    });

    describe('undefined NODE_ENV', () => {
        beforeEach(() => {
            delete process.env.NODE_ENV;
        });

        test('should behave as non-production (allow default secret)', () => {
            const result = validateJwtSecret(undefined, undefined);
            expect(result).toBe('topgun-secret-dev');
        });
    });

    describe('DEFAULT_JWT_SECRET constant', () => {
        test('should equal the expected default value', () => {
            expect(DEFAULT_JWT_SECRET).toBe('topgun-secret-dev');
        });
    });
});
