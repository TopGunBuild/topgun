/**
 * Phase 14D-3: Settings Page
 *
 * Provides UI for viewing and modifying server configuration.
 * Hot-reloadable settings can be changed without restart.
 */

import { useState, useEffect, useCallback } from 'react';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { adminFetch } from '@/lib/api';
import {
  Settings as SettingsIcon,
  Database,
  Server,
  Shield,
  Network,
  Gauge,
  Save,
  RotateCcw,
  Loader2,
  CheckCircle2,
  AlertCircle,
} from 'lucide-react';

interface SettingsData {
  general: {
    port: number;
    metricsPort: number;
    logLevel: string;
    version: string;
  };
  storage: {
    type: string;
    connectionString: string | null;
    status: string;
  };
  security: {
    jwtAlgorithm: string;
    sessionTimeout: number;
  };
  integrations: {
    mcp: { enabled: boolean; port: number };
    vectorSearch: { enabled: boolean; model: string | null };
  };
  cluster: {
    mode: string;
    nodeId: string;
    peers: string[];
    partitionCount: number;
  };
  rateLimits: {
    connections: number;
    messagesPerSecond: number;
  };
  _meta: {
    hotReloadable: string[];
    restartRequired: string[];
  };
}

type ToastType = 'success' | 'error' | 'info';

function Toast({
  message,
  type,
  onClose,
}: {
  message: string;
  type: ToastType;
  onClose: () => void;
}) {
  useEffect(() => {
    const timer = setTimeout(onClose, 5000);
    return () => clearTimeout(timer);
  }, [onClose]);

  return (
    <button
      onClick={onClose}
      className={`fixed bottom-4 right-4 px-4 py-3 rounded-lg shadow-lg flex items-center gap-2 z-50 cursor-pointer transition-opacity hover:opacity-90 ${
        type === 'success'
          ? 'bg-green-600 text-white'
          : type === 'error'
            ? 'bg-red-600 text-white'
            : 'bg-blue-600 text-white'
      }`}
    >
      {type === 'success' && <CheckCircle2 className="w-4 h-4" />}
      {type === 'error' && <AlertCircle className="w-4 h-4" />}
      <span>{message}</span>
      <span className="ml-2 text-xs opacity-75">×</span>
    </button>
  );
}

export function Settings() {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [changes, setChanges] = useState<Record<string, unknown>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<{ message: string; type: ToastType } | null>(null);

  const loadSettings = useCallback(async () => {
    try {
      const res = await adminFetch('/api/admin/settings');
      if (!res.ok) {
        throw new Error('Failed to load settings');
      }
      const data = await res.json();
      setSettings(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load settings');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  const handleChange = (path: string, value: unknown) => {
    setChanges((prev) => ({ ...prev, [path]: value }));
  };

  const handleSave = async () => {
    if (!hasChanges) return;

    setSaving(true);
    try {
      const body = unflattenObject(changes);

      // Validate first
      const validateRes = await adminFetch('/api/admin/settings/validate', {
        method: 'POST',
        body: JSON.stringify(body),
      });
      const validateData = await validateRes.json();

      if (!validateData.valid) {
        const errorMessages = validateData.errors
          .map((e: { path: string; message: string }) => `${e.path}: ${e.message}`)
          .join('; ');
        setToast({
          message: `Validation failed: ${errorMessages}`,
          type: 'error',
        });
        setSaving(false);
        return;
      }

      // Apply changes
      const res = await adminFetch('/api/admin/settings', {
        method: 'PATCH',
        body: JSON.stringify(body),
      });
      const data = await res.json();

      if (data.success) {
        setToast({
          message: `Updated: ${data.updated.join(', ')}`,
          type: 'success',
        });
        setChanges({});
        loadSettings();
      } else {
        setToast({
          message: data.message || data.error || 'Failed to save',
          type: 'error',
        });
      }
    } catch (err) {
      setToast({
        message: err instanceof Error ? err.message : 'Failed to save settings',
        type: 'error',
      });
    } finally {
      setSaving(false);
    }
  };

  const handleDiscard = () => {
    setChanges({});
  };

  const getValue = (path: string, defaultValue: unknown): unknown => {
    if (path in changes) {
      return changes[path];
    }
    return getNestedValue(settings, path) ?? defaultValue;
  };

  const hasChanges = Object.keys(changes).length > 0;

  if (loading) {
    return (
      <div className="p-6 flex items-center justify-center min-h-[400px]">
        <Loader2 className="w-8 h-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (error || !settings) {
    return (
      <div className="p-6">
        <Card className="border-destructive">
          <CardContent className="p-6">
            <div className="flex items-center gap-2 text-destructive">
              <AlertCircle className="w-5 h-5" />
              <span>{error || 'Failed to load settings'}</span>
            </div>
            <Button onClick={loadSettings} className="mt-4" variant="outline">
              <RotateCcw className="w-4 h-4 mr-2" />
              Retry
            </Button>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6 max-w-4xl mx-auto">
      {toast && (
        <Toast message={toast.message} type={toast.type} onClose={() => setToast(null)} />
      )}

      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <SettingsIcon className="w-6 h-6" />
            Settings
          </h1>
          <p className="text-muted-foreground">Server configuration and preferences</p>
        </div>
        {hasChanges && (
          <div className="flex gap-2">
            <Button variant="outline" onClick={handleDiscard}>
              <RotateCcw className="w-4 h-4 mr-2" />
              Discard
            </Button>
            <Button onClick={handleSave} disabled={saving}>
              {saving ? (
                <Loader2 className="w-4 h-4 mr-2 animate-spin" />
              ) : (
                <Save className="w-4 h-4 mr-2" />
              )}
              Save Changes
            </Button>
          </div>
        )}
      </div>

      <Tabs defaultValue="general">
        <TabsList className="grid w-full grid-cols-5">
          <TabsTrigger value="general" className="flex items-center gap-1">
            <Server className="w-4 h-4" />
            <span className="hidden sm:inline">General</span>
          </TabsTrigger>
          <TabsTrigger value="storage" className="flex items-center gap-1">
            <Database className="w-4 h-4" />
            <span className="hidden sm:inline">Storage</span>
          </TabsTrigger>
          <TabsTrigger value="integrations" className="flex items-center gap-1">
            <Shield className="w-4 h-4" />
            <span className="hidden sm:inline">Integrations</span>
          </TabsTrigger>
          <TabsTrigger value="cluster" className="flex items-center gap-1">
            <Network className="w-4 h-4" />
            <span className="hidden sm:inline">Cluster</span>
          </TabsTrigger>
          <TabsTrigger value="limits" className="flex items-center gap-1">
            <Gauge className="w-4 h-4" />
            <span className="hidden sm:inline">Limits</span>
          </TabsTrigger>
        </TabsList>

        {/* General Tab */}
        <TabsContent value="general" className="space-y-4 mt-4">
          <Card>
            <CardHeader>
              <CardTitle>Server Configuration</CardTitle>
              <CardDescription>Core server ports and logging</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                <div className="space-y-2">
                  <Label htmlFor="port">WebSocket Port</Label>
                  <Input id="port" value={settings.general.port} disabled />
                  <Badge variant="secondary" className="text-xs">
                    Restart required
                  </Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="metricsPort">Metrics Port</Label>
                  <Input id="metricsPort" value={settings.general.metricsPort} disabled />
                  <Badge variant="secondary" className="text-xs">
                    Restart required
                  </Badge>
                </div>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                <div className="space-y-2">
                  <Label htmlFor="logLevel">Log Level</Label>
                  <select
                    id="logLevel"
                    className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    value={getValue('logLevel', settings.general.logLevel) as string}
                    onChange={(e) => handleChange('logLevel', e.target.value)}
                  >
                    <option value="debug">Debug</option>
                    <option value="info">Info</option>
                    <option value="warn">Warn</option>
                    <option value="error">Error</option>
                  </select>
                  <Badge variant="outline" className="text-xs">
                    Hot-reloadable
                  </Badge>
                </div>

                <div className="space-y-2">
                  <Label>Prometheus Metrics</Label>
                  <div className="flex items-center gap-3 h-10">
                    <Switch
                      checked={getValue('metricsEnabled', true) as boolean}
                      onCheckedChange={(v) => handleChange('metricsEnabled', v)}
                    />
                    <span className="text-sm text-muted-foreground">
                      {getValue('metricsEnabled', true) ? 'Enabled' : 'Disabled'}
                    </span>
                  </div>
                  <Badge variant="outline" className="text-xs">
                    Hot-reloadable
                  </Badge>
                </div>
              </div>

              <div className="pt-4 border-t">
                <p className="text-sm text-muted-foreground">
                  Version: <span className="font-mono">{settings.general.version}</span>
                </p>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Storage Tab */}
        <TabsContent value="storage" className="space-y-4 mt-4">
          <Card>
            <CardHeader>
              <CardTitle>Storage Backend</CardTitle>
              <CardDescription>Database configuration and status</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <Label>Storage Type</Label>
                <Input value={settings.storage.type} disabled className="max-w-xs" />
                <Badge variant="secondary" className="text-xs">
                  Restart required
                </Badge>
              </div>

              {settings.storage.connectionString && (
                <div className="space-y-2">
                  <Label>Connection String</Label>
                  <Input value={settings.storage.connectionString} disabled type="password" />
                </div>
              )}

              <div className="flex items-center gap-2">
                <div
                  className={`w-2 h-2 rounded-full ${
                    settings.storage.status === 'connected'
                      ? 'bg-green-500'
                      : settings.storage.status === 'disconnected'
                        ? 'bg-yellow-500'
                        : 'bg-red-500'
                  }`}
                />
                <span className="text-sm">
                  {settings.storage.status === 'connected'
                    ? 'Connected'
                    : settings.storage.status === 'disconnected'
                      ? 'Disconnected'
                      : 'Error'}
                </span>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Integrations Tab */}
        <TabsContent value="integrations" className="space-y-4 mt-4">
          <Card>
            <CardHeader>
              <CardTitle>Model Context Protocol (MCP)</CardTitle>
              <CardDescription>
                Expose data to Claude Desktop and Cursor
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex items-center justify-between">
                <div>
                  <p className="font-medium">Enable MCP Server</p>
                  <p className="text-sm text-muted-foreground">
                    Port: {settings.integrations.mcp.port}
                  </p>
                </div>
                <Switch
                  checked={getValue('integrations.mcp.enabled', settings.integrations.mcp.enabled) as boolean}
                  onCheckedChange={(v) => handleChange('integrations.mcp.enabled', v)}
                />
              </div>
              <Badge variant="outline" className="text-xs mt-2">
                Hot-reloadable
              </Badge>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Vector Search</CardTitle>
              <CardDescription>Semantic AI search using embeddings</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex items-center justify-between">
                <div>
                  <p className="font-medium">Enable Vector Search</p>
                  <p className="text-sm text-muted-foreground">
                    Model: {settings.integrations.vectorSearch.model || 'Not configured'}
                  </p>
                </div>
                <Switch
                  checked={
                    getValue('integrations.vectorSearch.enabled', settings.integrations.vectorSearch.enabled) as boolean
                  }
                  onCheckedChange={(v) => handleChange('integrations.vectorSearch.enabled', v)}
                />
              </div>
              <Badge variant="outline" className="text-xs mt-2">
                Hot-reloadable
              </Badge>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Cluster Tab */}
        <TabsContent value="cluster" className="space-y-4 mt-4">
          <Card>
            <CardHeader>
              <CardTitle>Cluster Information</CardTitle>
              <CardDescription>Cluster topology and partition info</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label className="text-muted-foreground">Mode</Label>
                  <p className="font-medium capitalize">{settings.cluster.mode}</p>
                </div>
                <div>
                  <Label className="text-muted-foreground">Node ID</Label>
                  <p className="font-mono text-sm">{settings.cluster.nodeId}</p>
                </div>
                <div>
                  <Label className="text-muted-foreground">Total Partitions</Label>
                  <p className="font-medium">{settings.cluster.partitionCount}</p>
                </div>
                <div>
                  <Label className="text-muted-foreground">Peers</Label>
                  <p className="font-medium">
                    {settings.cluster.peers.length || 'None (standalone)'}
                  </p>
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Rate Limits Tab */}
        <TabsContent value="limits" className="space-y-4 mt-4">
          <Card>
            <CardHeader>
              <CardTitle>Rate Limits</CardTitle>
              <CardDescription>Connection and message throttling</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="space-y-2">
                <Label htmlFor="connections">Max Connections</Label>
                <Input
                  id="connections"
                  type="number"
                  min={1}
                  className="max-w-xs"
                  value={getValue('rateLimits.connections', settings.rateLimits.connections) as number}
                  onChange={(e) =>
                    handleChange('rateLimits.connections', parseInt(e.target.value, 10) || 0)
                  }
                />
                <Badge variant="outline" className="text-xs">
                  Hot-reloadable
                </Badge>
              </div>

              <div className="space-y-2">
                <Label htmlFor="messagesPerSecond">Messages per Second (per client)</Label>
                <Input
                  id="messagesPerSecond"
                  type="number"
                  min={1}
                  className="max-w-xs"
                  value={
                    getValue('rateLimits.messagesPerSecond', settings.rateLimits.messagesPerSecond) as number
                  }
                  onChange={(e) =>
                    handleChange('rateLimits.messagesPerSecond', parseInt(e.target.value, 10) || 0)
                  }
                />
                <Badge variant="outline" className="text-xs">
                  Hot-reloadable
                </Badge>
              </div>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}

/**
 * Convert flat dot-notation object to nested object
 */
function unflattenObject(obj: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = {};

  for (const [path, value] of Object.entries(obj)) {
    const keys = path.split('.');
    let current = result;

    for (let i = 0; i < keys.length - 1; i++) {
      const key = keys[i];
      if (!(key in current)) {
        current[key] = {};
      }
      current = current[key] as Record<string, unknown>;
    }

    current[keys[keys.length - 1]] = value;
  }

  return result;
}

/**
 * Get nested value from object by dot-notation path
 */
function getNestedValue(obj: unknown, path: string): unknown {
  if (!obj || typeof obj !== 'object') return undefined;

  const keys = path.split('.');
  let current: unknown = obj;

  for (const key of keys) {
    if (current === null || current === undefined || typeof current !== 'object') {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }

  return current;
}
