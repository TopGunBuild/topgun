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
import { useServerStatus } from './hooks/useServerStatus';

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

// Settings placeholder page
function Settings() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold mb-4">Settings</h1>
      <p className="text-muted-foreground">Server configuration and preferences.</p>
    </div>
  );
}

// Main App with Bootstrap Mode detection
function AppContent() {
  const { status, loading } = useServerStatus();
  const [initialized, setInitialized] = useState(false);

  // Initialize theme on mount
  useEffect(() => {
    const savedTheme = localStorage.getItem('theme');
    if (savedTheme === 'dark' || (!savedTheme && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
      document.documentElement.classList.add('dark');
    }
    setInitialized(true);
  }, []);

  // Show loading state
  if (loading || !initialized) {
    return (
      <div className="min-h-screen bg-background flex items-center justify-center">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary"></div>
      </div>
    );
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
