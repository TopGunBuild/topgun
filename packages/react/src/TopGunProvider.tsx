import React, { createContext, useContext, ReactNode } from 'react';
import { TopGunClient } from '@topgunbuild/client';

const TopGunContext = createContext<TopGunClient | null>(null);

export interface TopGunProviderProps {
  client: TopGunClient;
  children: ReactNode;
}

export const TopGunProvider: React.FC<TopGunProviderProps> = ({ client, children }) => {
  return (
    <TopGunContext.Provider value={client}>
      {children}
    </TopGunContext.Provider>
  );
};

export function useClient(): TopGunClient {
  const client = useContext(TopGunContext);
  if (!client) {
    throw new Error('useClient must be used within a TopGunProvider');
  }
  return client;
}

