import { useState, useEffect, useCallback } from 'react';

export interface ServerStatus {
  configured: boolean;
  version: string;
  mode: 'bootstrap' | 'normal';
}

const API_BASE = import.meta.env.VITE_API_URL || 'http://localhost:9090';

export function useServerStatus() {
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    try {
      setLoading(true);
      const res = await fetch(`${API_BASE}/api/status`);
      if (!res.ok) {
        throw new Error('Failed to fetch server status');
      }
      const data = await res.json();
      setStatus(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unknown error');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStatus();
  }, [fetchStatus]);

  return { status, loading, error, refetch: fetchStatus };
}
