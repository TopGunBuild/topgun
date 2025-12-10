import { renderHook } from '@testing-library/react';
import { useMutation } from '../hooks/useMutation';
import { TopGunProvider } from '../TopGunProvider';
import { TopGunClient } from '@topgunbuild/client';
import React from 'react';

// Mock TopGunClient
const mockSet = jest.fn();
const mockRemove = jest.fn();
const mockGetMap = jest.fn().mockReturnValue({
  set: mockSet,
  remove: mockRemove,
});

const mockClient = {
  getMap: mockGetMap,
} as unknown as TopGunClient;

describe('useMutation', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TopGunProvider client={mockClient}>{children}</TopGunProvider>
  );

  it('should return mutation functions', () => {
    const { result } = renderHook(() => useMutation('testMap'), { wrapper });

    expect(typeof result.current.create).toBe('function');
    expect(typeof result.current.update).toBe('function');
    expect(typeof result.current.remove).toBe('function');
    expect(result.current.map).toBeDefined();
  });

  it('should call map.set on create', () => {
    const { result } = renderHook(() => useMutation('testMap'), { wrapper });

    result.current.create('key1', { value: 'test' });
    expect(mockGetMap).toHaveBeenCalledWith('testMap');
    expect(mockSet).toHaveBeenCalledWith('key1', { value: 'test' });
  });

  it('should call map.set on update', () => {
    const { result } = renderHook(() => useMutation('testMap'), { wrapper });

    result.current.update('key1', { value: 'updated' });
    expect(mockSet).toHaveBeenCalledWith('key1', { value: 'updated' });
  });

  it('should call map.remove on remove', () => {
    const { result } = renderHook(() => useMutation('testMap'), { wrapper });

    result.current.remove('key1');
    expect(mockRemove).toHaveBeenCalledWith('key1');
  });
});

