/**
 * SWR global configuration for admin dashboard data fetching.
 * Replaces manual fetch+useState+setInterval polling patterns.
 */

import type { SWRConfiguration } from 'swr';
import { adminFetchJson } from './api';

/**
 * Default SWR fetcher using adminFetch with auth headers.
 * SWR passes the key (URL path) as the first argument.
 */
const fetcher = <T>(url: string): Promise<T> => adminFetchJson<T>(url);

/**
 * Global SWR configuration applied via <SWRConfig> in App.tsx
 */
export const swrConfig: SWRConfiguration = {
  fetcher,
  revalidateOnFocus: true,
  errorRetryCount: 3,
  errorRetryInterval: 5000,
  dedupingInterval: 2000,
};
