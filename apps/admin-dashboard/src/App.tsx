import React, { useEffect, useState } from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { TopGunProvider } from '@topgunbuild/react';
import { client } from './lib/client';
import { Login } from './pages/Login';
import { Dashboard } from './pages/Dashboard';
import { Maps } from './pages/Maps';
import { Layout } from './components/Layout';
import { ErrorBoundary } from './components/ErrorBoundary';
import { CommandPalette } from './components/CommandPalette';
import { SetupWizard } from './features/setup';
import { DataExplorer } from './features/explorer';
import { QueryPlayground } from './features/query';
import { ClusterTopology } from './features/cluster';
import { Settings } from './features/settings';
import { useServerStatus } from './hooks/useServerStatus';
import { ServerOff, RefreshCw } from 'lucide-react';
import { Button } from './components/ui/button';

// Protected Route Wrapper
const ProtectedRoute = ({ children }: { children: React.ReactNode }) => {
  const token = localStorage.getItem('topgun_token');

  // Check that token exists and has basic JWT format (3 dot-separated parts)
  const isValidFormat = token && token.split('.').length === 3;

  if (!isValidFormat) {
    // Clear invalid token if present
    if (token) localStorage.removeItem('topgun_token');
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
            Cannot connect to TopGun server. Make sure the server is running on port 8080
            with the admin API on port 9091.
          </p>
        </div>
        <div className="bg-muted/50 p-4 rounded-lg text-left text-sm font-mono">
          <p className="text-muted-foreground mb-2">Start the server:</p>
          <code>node bin/topgun.js dev</code>
        </div>
        <Button onClick={onRetry} variant="outline">
          <RefreshCw className="h-4 w-4 mr-2" />
          Retry Connection
        </Button>
      </div>
    </div>
  );
}

// Main App with Bootstrap Mode detection
function AppContent() {
  const { status, loading, error, refetch } = useServerStatus();
  const [initialized, setInitialized] = useState(false);

  // Initialize theme on mount
  useEffect(() => {
    const savedTheme = localStorage.getItem('theme');
    if (savedTheme === 'dark' || (!savedTheme && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
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

  // Show Setup Wizard if in bootstrap mode
  if (status?.mode === 'bootstrap') {
    return (
      <BrowserRouter>
        <Routes>
          <Route path="*" element={<SetupWizard onComplete={() => window.location.reload()} />} />
        </Routes>
      </BrowserRouter>
    );
  }

  return (
    <BrowserRouter>
      <CommandPalette />
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/setup" element={<SetupWizard onComplete={() => window.location.reload()} />} />

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
  return (
    <ErrorBoundary>
      <TopGunProvider client={client}>
        <AppContent />
      </TopGunProvider>
    </ErrorBoundary>
  );
}

export default App;
