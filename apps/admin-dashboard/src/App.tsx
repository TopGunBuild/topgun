import React from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { TopGunProvider } from '@topgunbuild/react';
import { client } from './lib/client';
import { Login } from './pages/Login';
import { Dashboard } from './pages/Dashboard';
import { Maps } from './pages/Maps';
import { Cluster } from './pages/Cluster';
import { Layout } from './components/Layout';
import { ErrorBoundary } from './components/ErrorBoundary';

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

function App() {
  return (
    <ErrorBoundary>
      <TopGunProvider client={client}>
        <BrowserRouter>
          <Routes>
            <Route path="/login" element={<Login />} />

            <Route path="/" element={
              <ProtectedRoute>
                <Layout />
              </ProtectedRoute>
            }>
              <Route index element={
                <ErrorBoundary>
                  <Dashboard />
                </ErrorBoundary>
              } />
              <Route path="maps" element={
                <ErrorBoundary>
                  <Maps />
                </ErrorBoundary>
              } />
              <Route path="cluster" element={
                <ErrorBoundary>
                  <Cluster />
                </ErrorBoundary>
              } />
            </Route>
          </Routes>
        </BrowserRouter>
      </TopGunProvider>
    </ErrorBoundary>
  );
}

export default App;
