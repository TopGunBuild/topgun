import { renderHook, act } from '@testing-library/react';
import { useQuery } from '../hooks/useQuery';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock TopGunClient
const mockSubscribe = jest.fn();
const mockQuery = jest.fn().mockReturnValue({
  subscribe: mockSubscribe,
});
const mockClient = {
  query: mockQuery,
} as unknown as TopGunClient;

describe('useQuery', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockSubscribe.mockReturnValue(() => {}); // Unsubscribe function
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should initialize with loading state', () => {
    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.loading).toBe(true);
    expect(result.current.data).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it('should update data when subscription fires', async () => {
    let callback: (results: any[]) => void;
    mockSubscribe.mockImplementation((cb) => {
      callback = cb;
      return () => {};
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.loading).toBe(true);

    act(() => {
      if (callback) {
        callback([{ _key: 'item-1', id: '1', text: 'test' }]);
      }
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.data).toEqual([{ _key: 'item-1', id: '1', text: 'test' }]);
  });

  it('should handle subscription errors', () => {
    mockQuery.mockImplementationOnce(() => {
      throw new Error('Subscription failed');
    });

    const { result } = renderHook(() => useQuery('testMap', {}), { wrapper });

    expect(result.current.error).toEqual(new Error('Subscription failed'));
    expect(result.current.loading).toBe(false);
  });
});

