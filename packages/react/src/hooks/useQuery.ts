import { useState, useEffect, useRef } from 'react';
import { QueryFilter, QueryResultItem } from '@topgunbuild/client';
import { useClient } from './useClient';

export interface UseQueryResult<T> {
  data: QueryResultItem<T>[];
  loading: boolean;
  error: Error | null;
}

export function useQuery<T = any>(mapName: string, query: QueryFilter = {}): UseQueryResult<T> {
  const client = useClient();
  const [data, setData] = useState<QueryResultItem<T>[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // Use a ref to track if the component is mounted to avoid state updates on unmounted components
  const isMounted = useRef(true);

  // We serialize the query to use it as a stable dependency for the effect
  const queryJson = JSON.stringify(query);

  useEffect(() => {
    isMounted.current = true;
    setLoading(true);
    
    try {
      const handle = client.query<T>(mapName, query);

      const unsubscribe = handle.subscribe((results) => {
        if (isMounted.current) {
          setData(results);
          setLoading(false);
        }
      });

      return () => {
        isMounted.current = false;
        unsubscribe();
      };
    } catch (err) {
      if (isMounted.current) {
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      }
      return () => {
        isMounted.current = false;
      };
    }
  }, [client, mapName, queryJson]);

  return { data, loading, error };
}

