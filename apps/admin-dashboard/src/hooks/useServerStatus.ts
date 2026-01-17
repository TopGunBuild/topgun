import { useState, useEffect, useCallback, useRef } from 'react';

export interface ServerStatus {
  configured: boolean;
  version: string;
  mode: 'bootstrap' | 'normal';
}

// Admin API runs on metrics port (9091), not main WebSocket port (8080)
const API_BASE = import.meta.env.VITE_API_URL || 'http://localhost:9091';

// Polling interval for connection status check (5 seconds)
const POLL_INTERVAL_MS = 5000;

export function useServerStatus() {
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const isFirstLoad = useRef(true);

  const fetchStatus = useCallback(async () => {
    try {
      // Only show loading spinner on first load, not on polling
      if (isFirstLoad.current) {
        setLoading(true);
      }
      const res = await fetch(`${API_BASE}/api/status`);
      if (!res.ok) {
        throw new Error('Failed to fetch server status');
      }
      const data = await res.json();
      setStatus(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unknown error');
      setStatus(null);
    } finally {
      setLoading(false);
      isFirstLoad.current = false;
    }
  }, []);

  useEffect(() => {
    fetchStatus();

    // Poll for connection status periodically
    const interval = setInterval(fetchStatus, POLL_INTERVAL_MS);

    return () => clearInterval(interval);
  }, [fetchStatus]);

  return { status, loading, error, refetch: fetchStatus };
}
