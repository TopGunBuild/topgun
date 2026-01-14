import { client, setAuthToken } from '@/lib/client';

/**
 * Hook providing access to the TopGun client instance
 */
export function useTopGun() {
  return {
    client,
    setAuthToken,
  };
}

export default useTopGun;
