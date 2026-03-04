/**
 * Settings page for viewing and modifying server configuration.
 * Uses SWR for data fetching and flat SettingsResponse from Rust admin API.
 */

import { useState, useEffect, useCallback } from 'react';
import useSWR from 'swr';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { adminFetch } from '@/lib/api';
import type { SettingsResponse, SettingsUpdateRequest, ErrorResponse } from '@/lib/admin-api-types';
import {
  Settings as SettingsIcon,
  Server,
  Shield,
  Save,
  RotateCcw,
  Loader2,
  CheckCircle2,
  AlertCircle,
} from 'lucide-react';

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
      <span className="ml-2 text-xs opacity-75">&times;</span>
    </button>
  );
}

export function Settings() {
  const { data: settings, error, isLoading, mutate } = useSWR<SettingsResponse>(
    '/api/admin/settings',
    { refreshInterval: 10000 }
  );

  const [changes, setChanges] = useState<Partial<SettingsUpdateRequest>>({});
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ message: string; type: ToastType } | null>(null);

  const handleChange = useCallback((field: keyof SettingsUpdateRequest, value: unknown) => {
    setChanges((prev) => ({ ...prev, [field]: value }));
  }, []);

  const handleSave = async () => {
    if (!hasChanges) return;

    setSaving(true);
    try {
      const res = await adminFetch('/api/admin/settings', {
        method: 'PUT',
        body: JSON.stringify(changes),
      });

      if (res.ok) {
        const updatedSettings: SettingsResponse = await res.json();
        mutate(updatedSettings, false);
        setChanges({});
        setToast({
          message: `Settings updated: ${Object.keys(changes).join(', ')}`,
          type: 'success',
        });
      } else {
        const errorData: ErrorResponse = await res.json();
        setToast({
          message: errorData.error || 'Failed to save settings',
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

  const getValue = <K extends keyof SettingsResponse>(field: K): SettingsResponse[K] | undefined => {
    if (field in changes) {
      return changes[field as keyof SettingsUpdateRequest] as SettingsResponse[K];
    }
    return settings?.[field];
  };

  const hasChanges = Object.keys(changes).length > 0;

  if (isLoading) {
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
              <span>{error instanceof Error ? error.message : 'Failed to load settings'}</span>
            </div>
            <Button onClick={() => mutate()} className="mt-4" variant="outline">
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

      <Tabs defaultValue="server">
        <TabsList className="grid w-full grid-cols-2">
          <TabsTrigger value="server" className="flex items-center gap-1">
            <Server className="w-4 h-4" />
            <span>Server</span>
          </TabsTrigger>
          <TabsTrigger value="integrations" className="flex items-center gap-1">
            <Shield className="w-4 h-4" />
            <span>Integrations</span>
          </TabsTrigger>
        </TabsList>

        {/* Server Tab */}
        <TabsContent value="server" className="space-y-4 mt-4">
          {/* Read-only server info */}
          <Card>
            <CardHeader>
              <CardTitle>Server Information</CardTitle>
              <CardDescription>Read-only server configuration (restart required to change)</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                <div className="space-y-2">
                  <Label htmlFor="nodeId">Node ID</Label>
                  <Input id="nodeId" value={settings.nodeId} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="host">Host</Label>
                  <Input id="host" value={settings.host} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="port">Port</Label>
                  <Input id="port" value={settings.port} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="partitionCount">Partition Count</Label>
                  <Input id="partitionCount" value={settings.partitionCount} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="requireAuth">Require Auth</Label>
                  <Input id="requireAuth" value={settings.requireAuth ? 'Yes' : 'No'} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="maxValueBytes">Max Value Bytes</Label>
                  <Input id="maxValueBytes" value={settings.maxValueBytes} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="defaultOperationTimeoutMs">Default Operation Timeout (ms)</Label>
                  <Input id="defaultOperationTimeoutMs" value={settings.defaultOperationTimeoutMs} disabled />
                  <Badge variant="secondary" className="text-xs">Restart required</Badge>
                </div>
              </div>
            </CardContent>
          </Card>

          {/* Editable settings */}
          <Card>
            <CardHeader>
              <CardTitle>Runtime Configuration</CardTitle>
              <CardDescription>Settings that can be changed without restart</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                <div className="space-y-2">
                  <Label htmlFor="logLevel">Log Level</Label>
                  <select
                    id="logLevel"
                    className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    value={(getValue('logLevel') as string) ?? 'info'}
                    onChange={(e) => handleChange('logLevel', e.target.value)}
                  >
                    <option value="trace">Trace</option>
                    <option value="debug">Debug</option>
                    <option value="info">Info</option>
                    <option value="warn">Warn</option>
                    <option value="error">Error</option>
                  </select>
                  <Badge variant="outline" className="text-xs">Hot-reloadable</Badge>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="gcIntervalMs">GC Interval (ms)</Label>
                  <Input
                    id="gcIntervalMs"
                    type="number"
                    min={1000}
                    value={(getValue('gcIntervalMs') as number) ?? 0}
                    onChange={(e) => handleChange('gcIntervalMs', parseInt(e.target.value, 10) || 0)}
                  />
                  <Badge variant="outline" className="text-xs">Hot-reloadable</Badge>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="maxConcurrentOperations">Max Concurrent Operations</Label>
                  <Input
                    id="maxConcurrentOperations"
                    type="number"
                    min={1}
                    value={(getValue('maxConcurrentOperations') as number) ?? 0}
                    onChange={(e) => handleChange('maxConcurrentOperations', parseInt(e.target.value, 10) || 0)}
                  />
                  <Badge variant="outline" className="text-xs">Hot-reloadable</Badge>
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Integrations Tab */}
        <TabsContent value="integrations" className="space-y-4 mt-4">
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
                    Not yet available in the Rust server
                  </p>
                </div>
                <Switch disabled checked={false} />
              </div>
              <Badge variant="secondary" className="text-xs mt-2">Coming soon</Badge>
            </CardContent>
          </Card>

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
                    Not yet available in the Rust server
                  </p>
                </div>
                <Switch disabled checked={false} />
              </div>
              <Badge variant="secondary" className="text-xs mt-2">Coming soon</Badge>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}
