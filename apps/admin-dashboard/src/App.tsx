import React, { useEffect, useState } from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { TopGunProvider } from '@topgunbuild/react';
import { client } from './lib/client';
import { swrConfig } from './lib/swr-config';
import { getAuthStatus } from './lib/api';
import { Login } from './pages/Login';
import { Dashboard } from './pages/Dashboard';
import { Maps } from './pages/Maps';
import { Layout } from './components/Layout';
import { ErrorBoundary } from './components/ErrorBoundary';
import { CommandPalette } from './components/CommandPalette';
import { DataExplorer } from './features/explorer';
import { QueryPlayground } from './features/query';
import { ClusterTopology } from './features/cluster';
import { Settings } from './features/settings';
import { useServerStatus } from './hooks/useServerStatus';
import { ServerOff, RefreshCw } from 'lucide-react';
import { Button } from './components/ui/button';

// Auth status is fetched once at app start and shared via context so every
// ProtectedRoute and the Login page can act on it without re-fetching.
interface AuthStatusContextValue {
  authRequired: boolean;
  loading: boolean;
}

export const AuthStatusContext = React.createContext<AuthStatusContextValue>({
  authRequired: true,
  loading: true,
});

// Protected Route Wrapper
const ProtectedRoute = ({ children }: { children: React.ReactNode }) => {
  const { authRequired, loading } = React.useContext(AuthStatusContext);

  // Wait until auth posture is known to avoid a flash of the login form.
  if (loading) {
    return (
      <div className="min-h-screen bg-background flex items-center justify-center">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary"></div>
      </div>
    );
  }

  // When auth is not required, allow through without a token.
  if (!authRequired) {
    return <>{children}</>;
  }

  const token = localStorage.getItem('topgun_admin_token');

  // Check that token exists and has basic JWT format (3 dot-separated parts)
  const isValidFormat = token && token.split('.').length === 3;

  if (!isValidFormat) {
    // Clear invalid token if present
    if (token) localStorage.removeItem('topgun_admin_token');
    return <Navigate to="/login" replace />;
  }

  return <>{children}</>;
};

// Server Unavailable Screen
function ServerUnavailable({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="min-h-screen bg-background flex items-center justify-center p-4">
      <div className="text-center space-y-6 max-w-md">
        <div className="w-16 h-16 mx-auto rounded-full bg-destructive/10 flex items-center justify-center">
          <ServerOff className="h-8 w-8 text-destructive" />
        </div>
        <div>
          <h1 className="text-2xl font-bold mb-2">Server Unavailable</h1>
          <p className="text-muted-foreground">
            Cannot connect to TopGun server. Make sure the server is running.
          </p>
        </div>
        <div className="bg-muted/50 p-4 rounded-lg text-left text-sm font-mono">
          <p className="text-muted-foreground mb-2">Start the server:</p>
          <code>cargo run --bin topgun-server</code>
        </div>
        <Button onClick={onRetry} variant="outline">
          <RefreshCw className="h-4 w-4 mr-2" />
          Retry Connection
        </Button>
      </div>
    </div>
  );
}

// Main App
function AppContent() {
  const { status, loading, error, refetch } = useServerStatus();
  const [initialized, setInitialized] = useState(false);

  // Initialize theme on mount
  useEffect(() => {
    const savedTheme = localStorage.getItem('theme');
    if (
      savedTheme === 'dark' ||
      (!savedTheme && window.matchMedia('(prefers-color-scheme: dark)').matches)
    ) {
      document.documentElement.classList.add('dark');
    }
    setInitialized(true);
  }, []);

  // Show loading state (only on initial load)
  if (loading || !initialized) {
    return (
      <div className="min-h-screen bg-background flex items-center justify-center">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary"></div>
      </div>
    );
  }

  // Show Server Unavailable screen if connection failed
  if (error && !status) {
    return <ServerUnavailable onRetry={refetch} />;
  }

  return (
    <BrowserRouter basename="/admin">
      <CommandPalette />
      <Routes>
        <Route path="/login" element={<Login />} />

        <Route
          path="/"
          element={
            <ProtectedRoute>
              <Layout />
            </ProtectedRoute>
          }
        >
          <Route
            index
            element={
              <ErrorBoundary>
                <Dashboard />
              </ErrorBoundary>
            }
          />
          <Route
            path="maps"
            element={
              <ErrorBoundary>
                <Maps />
              </ErrorBoundary>
            }
          />
          <Route
            path="explorer"
            element={
              <ErrorBoundary>
                <DataExplorer />
              </ErrorBoundary>
            }
          />
          <Route
            path="playground"
            element={
              <ErrorBoundary>
                <QueryPlayground />
              </ErrorBoundary>
            }
          />
          <Route
            path="cluster"
            element={
              <ErrorBoundary>
                <ClusterTopology />
              </ErrorBoundary>
            }
          />
          <Route
            path="settings"
            element={
              <ErrorBoundary>
                <Settings />
              </ErrorBoundary>
            }
          />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}

function App() {
  const [authStatus, setAuthStatus] = useState<AuthStatusContextValue>({
    authRequired: true,
    loading: true,
  });

  // Fetch auth posture once on mount. Default is authRequired:true (fail-safe)
  // so that a network error shows the Login form rather than bypassing it.
  useEffect(() => {
    getAuthStatus().then((result) => {
      setAuthStatus({ authRequired: result.authRequired, loading: false });
    });
  }, []);

  return (
    <ErrorBoundary>
      <AuthStatusContext.Provider value={authStatus}>
        <SWRConfig value={swrConfig}>
          <TopGunProvider client={client}>
            <AppContent />
          </TopGunProvider>
        </SWRConfig>
      </AuthStatusContext.Provider>
    </ErrorBoundary>
  );
}

export default App;
