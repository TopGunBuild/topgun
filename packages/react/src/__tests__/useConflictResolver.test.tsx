import { renderHook, act, waitFor } from '@testing-library/react';
import { useConflictResolver } from '../hooks/useConflictResolver';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock ConflictResolverClient
const createMockConflictResolvers = () => ({
  register: jest.fn().mockResolvedValue({ success: true }),
  unregister: jest.fn().mockResolvedValue({ success: true }),
  list: jest.fn().mockResolvedValue([]),
  onRejection: jest.fn().mockReturnValue(() => {}),
});

let mockConflictResolvers: ReturnType<typeof createMockConflictResolvers>;

const mockClient = {
  getConflictResolvers: jest.fn(() => mockConflictResolvers),
} as unknown as TopGunClient;

describe('useConflictResolver', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockConflictResolvers = createMockConflictResolvers();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  describe('Initialization', () => {
    it('should return register, unregister, list functions', () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      expect(typeof result.current.register).toBe('function');
      expect(typeof result.current.unregister).toBe('function');
      expect(typeof result.current.list).toBe('function');
    });

    it('should initialize with loading=false, error=null, registered=[]', () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      expect(result.current.loading).toBe(false);
      expect(result.current.error).toBe(null);
      expect(result.current.registered).toEqual([]);
    });
  });

  describe('register()', () => {
    it('should call client.getConflictResolvers().register() with mapName and resolver', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept", value: context.remoteValue };',
        priority: 100,
      };

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(mockConflictResolvers.register).toHaveBeenCalledWith('testMap', resolver);
    });

    it('should set loading=true during registration', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      mockConflictResolvers.register.mockImplementation(() =>
        new Promise(resolve => setTimeout(() => resolve({ success: true }), 100))
      );

      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept" };',
        priority: 100,
      };

      let registerPromise: Promise<any>;
      act(() => {
        registerPromise = result.current.register(resolver);
      });

      // Check loading is true while promise is pending
      expect(result.current.loading).toBe(true);

      await act(async () => {
        await registerPromise!;
      });

      // Check loading is false after completion
      expect(result.current.loading).toBe(false);
    });

    it('should add resolver name to registered array on success', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept" };',
        priority: 100,
      };

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(result.current.registered).toContain('test-resolver');
    });

    it('should set error on failure', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      mockConflictResolvers.register.mockResolvedValue({
        success: false,
        error: 'Registration failed'
      });

      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept" };',
        priority: 100,
      };

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(result.current.error).not.toBe(null);
      expect(result.current.error?.message).toBe('Registration failed');
    });

    it('should not duplicate names in registered array', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept" };',
        priority: 100,
      };

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(result.current.registered).toEqual(['test-resolver']);

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(result.current.registered).toEqual(['test-resolver']);
    });
  });

  describe('unregister()', () => {
    it('should call client.getConflictResolvers().unregister() with mapName and resolverName', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      await act(async () => {
        await result.current.unregister('test-resolver');
      });

      expect(mockConflictResolvers.unregister).toHaveBeenCalledWith('testMap', 'test-resolver');
    });

    it('should remove resolver name from registered array on success', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      // Register first
      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept" };',
        priority: 100,
      };

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(result.current.registered).toContain('test-resolver');

      // Now unregister
      await act(async () => {
        await result.current.unregister('test-resolver');
      });

      expect(result.current.registered).not.toContain('test-resolver');
    });

    it('should set error on failure', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      mockConflictResolvers.unregister.mockResolvedValue({
        success: false,
        error: 'Unregistration failed'
      });

      await act(async () => {
        await result.current.unregister('test-resolver');
      });

      expect(result.current.error).not.toBe(null);
      expect(result.current.error?.message).toBe('Unregistration failed');
    });
  });

  describe('list()', () => {
    it('should call client.getConflictResolvers().list() with mapName', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      await act(async () => {
        await result.current.list();
      });

      expect(mockConflictResolvers.list).toHaveBeenCalledWith('testMap');
    });

    it('should return resolver info array', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      const resolverInfo = [
        { name: 'resolver1', priority: 100 },
        { name: 'resolver2', priority: 90 },
      ];
      mockConflictResolvers.list.mockResolvedValue(resolverInfo);

      let listResult: any;
      await act(async () => {
        listResult = await result.current.list();
      });

      expect(listResult).toEqual(resolverInfo);
    });

    it('should set error on failure', async () => {
      const { result } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      mockConflictResolvers.list.mockRejectedValue(new Error('List failed'));

      let listResult: any;
      await act(async () => {
        listResult = await result.current.list();
      });

      expect(result.current.error).not.toBe(null);
      expect(result.current.error?.message).toBe('List failed');
      expect(listResult).toEqual([]);
    });
  });

  describe('Auto-unregister', () => {
    it('should unregister all registered resolvers on unmount when autoUnregister=true (default)', async () => {
      const { result, unmount } = renderHook(() => useConflictResolver('testMap'), { wrapper });

      // Register two resolvers
      await act(async () => {
        await result.current.register({
          name: 'resolver1',
          code: 'return { action: "accept" };',
          priority: 100
        });
        await result.current.register({
          name: 'resolver2',
          code: 'return { action: "accept" };',
          priority: 90
        });
      });

      expect(result.current.registered).toEqual(['resolver1', 'resolver2']);

      // Clear mock to verify unmount calls
      mockConflictResolvers.unregister.mockClear();

      unmount();

      await waitFor(() => {
        expect(mockConflictResolvers.unregister).toHaveBeenCalledWith('testMap', 'resolver1');
        expect(mockConflictResolvers.unregister).toHaveBeenCalledWith('testMap', 'resolver2');
      });
    });

    it('should NOT unregister on unmount when autoUnregister=false', async () => {
      const { result, unmount } = renderHook(
        () => useConflictResolver('testMap', { autoUnregister: false }),
        { wrapper }
      );

      // Register a resolver
      await act(async () => {
        await result.current.register({
          name: 'resolver1',
          code: 'return { action: "accept" };',
          priority: 100
        });
      });

      expect(result.current.registered).toEqual(['resolver1']);

      // Clear mock to verify unmount does not call unregister
      mockConflictResolvers.unregister.mockClear();

      unmount();

      await waitFor(() => {
        expect(mockConflictResolvers.unregister).not.toHaveBeenCalled();
      });
    });
  });

  describe('Map name changes', () => {
    it('should use new mapName when prop changes', async () => {
      const { result, rerender } = renderHook(
        ({ mapName }) => useConflictResolver(mapName),
        { wrapper, initialProps: { mapName: 'map1' } }
      );

      const resolver = {
        name: 'test-resolver',
        code: 'return { action: "accept" };',
        priority: 100,
      };

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(mockConflictResolvers.register).toHaveBeenCalledWith('map1', resolver);

      rerender({ mapName: 'map2' });

      mockConflictResolvers.register.mockClear();

      await act(async () => {
        await result.current.register(resolver);
      });

      expect(mockConflictResolvers.register).toHaveBeenCalledWith('map2', resolver);
    });
  });
});
