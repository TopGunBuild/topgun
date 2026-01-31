import { renderHook, act } from '@testing-library/react';
import { useEntryProcessor } from '../hooks/useEntryProcessor';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

const mockExecuteOnKey = jest.fn().mockResolvedValue({ success: true, result: 42 });
const mockExecuteOnKeys = jest.fn().mockResolvedValue(new Map([['key1', { success: true }]]));

const mockClient = {
  executeOnKey: mockExecuteOnKey,
  executeOnKeys: mockExecuteOnKeys,
} as unknown as TopGunClient;

describe('useEntryProcessor', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  const processorDef = {
    name: 'increment',
    code: 'const current = value ?? 0; return { value: current + 1, result: current + 1 };',
  };

  describe('Initialization', () => {
    it('should return execute, executeMany, reset functions', () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      expect(typeof result.current.execute).toBe('function');
      expect(typeof result.current.executeMany).toBe('function');
      expect(typeof result.current.reset).toBe('function');
    });

    it('should initialize with executing=false, lastResult=null, error=null', () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      expect(result.current.executing).toBe(false);
      expect(result.current.lastResult).toBe(null);
      expect(result.current.error).toBe(null);
    });
  });

  describe('execute()', () => {
    it('should call client.executeOnKey() with mapName, key, and processor with args', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      await act(async () => {
        await result.current.execute('key1', { userId: 'user123' });
      });

      expect(mockExecuteOnKey).toHaveBeenCalledWith('testMap', 'key1', {
        ...processorDef,
        args: { userId: 'user123' },
      });
    });

    it('should set executing=true during execution', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      let resolver: any;
      mockExecuteOnKey.mockImplementation(() =>
        new Promise(resolve => { resolver = resolve; })
      );

      let executePromise: Promise<any>;
      act(() => {
        executePromise = result.current.execute('key1');
      });

      // Check executing is true while promise is pending
      expect(result.current.executing).toBe(true);

      await act(async () => {
        resolver({ success: true, result: 42 });
        await executePromise!;
      });

      // Check executing is false after completion
      expect(result.current.executing).toBe(false);
    });

    it('should update lastResult on success', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      const expectedResult = { success: true, result: 42 };
      mockExecuteOnKey.mockResolvedValue(expectedResult);

      await act(async () => {
        await result.current.execute('key1');
      });

      expect(result.current.lastResult).toEqual(expectedResult);
    });

    it('should set error on failure and throw', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      const error = new Error('Execution failed');
      mockExecuteOnKey.mockRejectedValue(error);

      await act(async () => {
        await expect(result.current.execute('key1')).rejects.toThrow('Execution failed');
      });

      expect(result.current.error).toEqual(error);
    });
  });

  describe('executeMany()', () => {
    it('should call client.executeOnKeys() with mapName, keys array, and processor', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      const keys = ['key1', 'key2', 'key3'];

      await act(async () => {
        await result.current.executeMany(keys, { batch: true });
      });

      expect(mockExecuteOnKeys).toHaveBeenCalledWith('testMap', keys, {
        ...processorDef,
        args: { batch: true },
      });
    });

    it('should set executing=true during execution', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      let resolver: any;
      mockExecuteOnKeys.mockImplementation(() =>
        new Promise(resolve => { resolver = resolve; })
      );

      let executeManyPromise: Promise<any>;
      act(() => {
        executeManyPromise = result.current.executeMany(['key1']);
      });

      // Check executing is true while promise is pending
      expect(result.current.executing).toBe(true);

      await act(async () => {
        resolver(new Map());
        await executeManyPromise!;
      });

      // Check executing is false after completion
      expect(result.current.executing).toBe(false);
    });

    it('should return Map of results', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      const expectedResults = new Map([
        ['key1', { success: true, result: 1 }],
        ['key2', { success: true, result: 2 }],
      ]);
      mockExecuteOnKeys.mockResolvedValue(expectedResults);

      let results: any;
      await act(async () => {
        results = await result.current.executeMany(['key1', 'key2']);
      });

      expect(results).toEqual(expectedResults);
    });

    it('should set error on failure and throw', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      const error = new Error('ExecuteMany failed');
      mockExecuteOnKeys.mockRejectedValue(error);

      await act(async () => {
        await expect(result.current.executeMany(['key1'])).rejects.toThrow('ExecuteMany failed');
      });

      expect(result.current.error).toEqual(error);
    });
  });

  describe('Retry logic', () => {
    it('should retry on failure when retries > 0', async () => {
      const { result } = renderHook(
        () => useEntryProcessor('testMap', processorDef, { retries: 2, retryDelayMs: 10 }),
        { wrapper }
      );

      let callCount = 0;
      mockExecuteOnKey.mockImplementation(() => {
        callCount++;
        if (callCount < 3) {
          return Promise.reject(new Error('Temporary failure'));
        }
        return Promise.resolve({ success: true, result: 42 });
      });

      await act(async () => {
        await result.current.execute('key1');
      });

      expect(callCount).toBe(3);
      expect(result.current.lastResult).toEqual({ success: true, result: 42 });
    });

    it('should use exponential backoff (retryDelayMs * 2^attempt)', async () => {
      const { result } = renderHook(
        () => useEntryProcessor('testMap', processorDef, { retries: 3, retryDelayMs: 10 }),
        { wrapper }
      );

      const delays: number[] = [];
      const originalSetTimeout = global.setTimeout;
      jest.spyOn(global, 'setTimeout').mockImplementation(((callback: any, delay: number) => {
        if (delay > 0) delays.push(delay);
        return originalSetTimeout(callback, 0) as any;
      }) as any);

      mockExecuteOnKey.mockRejectedValue(new Error('Always fails'));

      await act(async () => {
        await result.current.execute('key1').catch(() => {});
      });

      // Exponential backoff: 10, 20, 40
      expect(delays).toEqual([10, 20, 40]);

      jest.spyOn(global, 'setTimeout').mockRestore();
    });

    it('should set error after all retries exhausted', async () => {
      const { result } = renderHook(
        () => useEntryProcessor('testMap', processorDef, { retries: 2, retryDelayMs: 10 }),
        { wrapper }
      );

      const error = new Error('Persistent failure');
      mockExecuteOnKey.mockRejectedValue(error);

      await act(async () => {
        await expect(result.current.execute('key1')).rejects.toThrow('Persistent failure');
      });

      expect(result.current.error).toEqual(error);
      // Should have tried: initial + 2 retries = 3 total
      expect(mockExecuteOnKey).toHaveBeenCalledTimes(3);
    });
  });

  describe('reset()', () => {
    it('should clear lastResult and error', async () => {
      const { result } = renderHook(() => useEntryProcessor('testMap', processorDef), { wrapper });

      // Reset mock to ensure clean state
      mockExecuteOnKey.mockResolvedValue({ success: true, result: 42 });

      // Execute to populate state
      await act(async () => {
        await result.current.execute('key1');
      });

      expect(result.current.lastResult).not.toBe(null);

      // Now reset
      act(() => {
        result.current.reset();
      });

      expect(result.current.lastResult).toBe(null);
      expect(result.current.error).toBe(null);
    });
  });

  describe('Processor definition stability', () => {
    it('should use latest processorDef without re-creating callbacks', async () => {
      const processor1 = {
        name: 'processor1',
        code: 'return { value: 1, result: 1 };',
      };

      const processor2 = {
        name: 'processor2',
        code: 'return { value: 2, result: 2 };',
      };

      // Reset mock to ensure clean state
      mockExecuteOnKey.mockResolvedValue({ success: true, result: 1 });

      const { result, rerender } = renderHook(
        ({ proc }) => useEntryProcessor('testMap', proc),
        { wrapper, initialProps: { proc: processor1 } }
      );

      const executeRef1 = result.current.execute;

      await act(async () => {
        await result.current.execute('key1');
      });

      expect(mockExecuteOnKey).toHaveBeenCalledWith('testMap', 'key1', {
        ...processor1,
        args: undefined,
      });

      rerender({ proc: processor2 });

      const executeRef2 = result.current.execute;

      // Function reference should change when processorDef changes
      expect(executeRef1).not.toBe(executeRef2);

      mockExecuteOnKey.mockClear();

      await act(async () => {
        await result.current.execute('key1');
      });

      expect(mockExecuteOnKey).toHaveBeenCalledWith('testMap', 'key1', {
        ...processor2,
        args: undefined,
      });
    });
  });
});
