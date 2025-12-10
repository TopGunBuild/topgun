import { renderHook } from '@testing-library/react';
import { useClient } from '../hooks/useClient';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock TopGunClient
const mockClient = {
  query: jest.fn(),
  getMap: jest.fn(),
  getORMap: jest.fn(),
  topic: jest.fn(),
  getLock: jest.fn(),
  start: jest.fn(),
  setAuthToken: jest.fn(),
  setAuthTokenProvider: jest.fn(),
} as unknown as TopGunClient;

describe('useClient', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should return the TopGunClient from context', () => {
    const { result } = renderHook(() => useClient(), { wrapper });

    expect(result.current).toBe(mockClient);
  });

  it('should throw error when used outside of TopGunProvider', () => {
    // Suppress console.error for this test as React will log the error
    const consoleSpy = jest.spyOn(console, 'error').mockImplementation(() => {});

    expect(() => {
      renderHook(() => useClient());
    }).toThrow('useClient must be used within a TopGunProvider');

    consoleSpy.mockRestore();
  });

  it('should return the same client instance on re-renders', () => {
    const { result, rerender } = renderHook(() => useClient(), { wrapper });

    const firstClient = result.current;

    rerender();

    expect(result.current).toBe(firstClient);
  });

  it('should provide access to client methods', () => {
    const { result } = renderHook(() => useClient(), { wrapper });

    expect(typeof result.current.query).toBe('function');
    expect(typeof result.current.getMap).toBe('function');
    expect(typeof result.current.getORMap).toBe('function');
    expect(typeof result.current.topic).toBe('function');
    expect(typeof result.current.getLock).toBe('function');
  });

  it('should work with nested providers (uses closest provider)', () => {
    const outerClient = {
      ...mockClient,
      _id: 'outer',
    } as unknown as TopGunClient;

    const innerClient = {
      ...mockClient,
      _id: 'inner',
    } as unknown as TopGunClient;

    const nestedWrapper = ({ children }: { children: React.ReactNode }) => (
      <TopGunProvider client={outerClient}>
        <TopGunProvider client={innerClient}>
          {children}
        </TopGunProvider>
      </TopGunProvider>
    );

    const { result } = renderHook(() => useClient(), { wrapper: nestedWrapper });

    // Should get the inner (closest) provider's client
    expect((result.current as any)._id).toBe('inner');
  });
});
