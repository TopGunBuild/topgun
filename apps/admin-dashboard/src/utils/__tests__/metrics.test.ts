import { getMetricValue, formatBytes, createLookupByKey } from '../metrics';

describe('getMetricValue', () => {
    describe('with number input', () => {
        it('should return the number as-is', () => {
            expect(getMetricValue(42)).toBe(42);
            expect(getMetricValue(0)).toBe(0);
            expect(getMetricValue(123.456)).toBe(123.456);
        });

        it('should handle negative numbers', () => {
            expect(getMetricValue(-10)).toBe(-10);
        });
    });

    describe('with array of labeled metrics', () => {
        it('should sum all values in the array', () => {
            const metrics = [
                { value: 50, labels: { type: 'PUT' } },
                { value: 100, labels: { type: 'GET' } },
            ];
            expect(getMetricValue(metrics)).toBe(150);
        });

        it('should handle single item array', () => {
            const metrics = [{ value: 42, labels: { type: 'DELETE' } }];
            expect(getMetricValue(metrics)).toBe(42);
        });

        it('should handle empty array', () => {
            expect(getMetricValue([])).toBe(0);
        });

        it('should handle items without value', () => {
            const metrics = [
                { value: 50 },
                { labels: { type: 'GET' } }, // missing value
                { value: 30 },
            ];
            expect(getMetricValue(metrics)).toBe(80);
        });

        it('should handle null items in array', () => {
            const metrics = [{ value: 50 }, null, { value: 30 }];
            expect(getMetricValue(metrics)).toBe(80);
        });
    });

    describe('with invalid input', () => {
        it('should return 0 for undefined', () => {
            expect(getMetricValue(undefined)).toBe(0);
        });

        it('should return 0 for null', () => {
            expect(getMetricValue(null)).toBe(0);
        });

        it('should return 0 for string', () => {
            expect(getMetricValue('42')).toBe(0);
        });

        it('should return 0 for object', () => {
            expect(getMetricValue({ value: 42 })).toBe(0);
        });
    });
});

describe('formatBytes', () => {
    it('should format 0 bytes', () => {
        expect(formatBytes(0)).toBe('0 B');
    });

    it('should format bytes (< 1 KB)', () => {
        expect(formatBytes(512)).toBe('512.0 B');
        expect(formatBytes(1)).toBe('1.0 B');
    });

    it('should format kilobytes', () => {
        expect(formatBytes(1024)).toBe('1.0 KB');
        expect(formatBytes(1536)).toBe('1.5 KB');
        expect(formatBytes(10240)).toBe('10.0 KB');
    });

    it('should format megabytes', () => {
        expect(formatBytes(1048576)).toBe('1.0 MB');
        expect(formatBytes(1572864)).toBe('1.5 MB');
        expect(formatBytes(52428800)).toBe('50.0 MB');
        expect(formatBytes(157286400)).toBe('150.0 MB');
    });

    it('should format gigabytes', () => {
        expect(formatBytes(1073741824)).toBe('1.0 GB');
        expect(formatBytes(2147483648)).toBe('2.0 GB');
    });
});

describe('createLookupByKey', () => {
    it('should create lookup map from array with _key', () => {
        const items = [
            { _key: 'node-1', name: 'Node 1', value: 100 },
            { _key: 'node-2', name: 'Node 2', value: 200 },
        ];

        const lookup = createLookupByKey(items);

        expect(lookup['node-1']).toEqual({ _key: 'node-1', name: 'Node 1', value: 100 });
        expect(lookup['node-2']).toEqual({ _key: 'node-2', name: 'Node 2', value: 200 });
    });

    it('should handle empty array', () => {
        const lookup = createLookupByKey([]);
        expect(lookup).toEqual({});
    });

    it('should skip items without _key', () => {
        const items = [
            { _key: 'node-1', name: 'Node 1' },
            { _key: '', name: 'Empty key' }, // empty string is falsy
            { _key: 'node-3', name: 'Node 3' },
        ] as { _key: string; name: string }[];

        const lookup = createLookupByKey(items);

        expect(Object.keys(lookup)).toHaveLength(2);
        expect(lookup['node-1']).toBeDefined();
        expect(lookup['node-3']).toBeDefined();
    });

    it('should overwrite duplicate keys with last value', () => {
        const items = [
            { _key: 'node-1', name: 'First' },
            { _key: 'node-1', name: 'Second' },
        ];

        const lookup = createLookupByKey(items);

        expect(lookup['node-1'].name).toBe('Second');
    });
});
