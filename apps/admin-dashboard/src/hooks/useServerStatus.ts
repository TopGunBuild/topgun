import useSWR from 'swr';
import { API_BASE } from '@/lib/api';
import type { ServerStatusResponse } from '@/lib/admin-api-types';

/**
 * Hook to fetch server status using SWR with automatic polling.
 * Replaces manual fetch+useState+setInterval pattern.
 */
export function useServerStatus() {
  const { data, error, isLoading, mutate } = useSWR<ServerStatusResponse>(
    `${API_BASE}/api/status`,
    {
      refreshInterval: 5000,
      // The /api/status endpoint is public, so use a plain fetcher without auth
      fetcher: async (url: string) => {
        const res = await fetch(url);
        if (!res.ok) throw new Error('Failed to fetch server status');
        return res.json();
      },
    }
  );

  return {
    status: data ?? null,
    loading: isLoading,
    error: error ? (error instanceof Error ? error.message : 'Unknown error') : null,
    refetch: () => mutate(),
  };
}
