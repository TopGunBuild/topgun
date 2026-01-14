import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Progress } from '@/components/ui/progress';
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group';
import { Switch } from '@/components/ui/switch';
import { cn } from '@/lib/utils';
import { Loader2, Check, X, Download, Rocket } from 'lucide-react';

interface SetupConfig {
  deploymentMode: 'standalone' | 'cluster';
  storage: {
    type: 'sqlite' | 'postgres' | 'memory';
    connectionString?: string;
    dataDir?: string;
  };
  admin: {
    username: string;
    password: string;
    email?: string;
  };
  server: {
    port: number;
    metricsPort: number;
  };
  integrations: {
    mcpEnabled: boolean;
    mcpPort?: number;
    vectorSearchEnabled: boolean;
    vectorModel?: string;
  };
}

const TOTAL_STEPS = 6;
const API_BASE = import.meta.env.VITE_API_URL || 'http://localhost:9090';

export function SetupWizard({ onComplete }: { onComplete: () => void }) {
  const [step, setStep] = useState(1);
  const [config, setConfig] = useState<Partial<SetupConfig>>({
    deploymentMode: 'standalone',
    storage: { type: 'sqlite', dataDir: './data' },
    server: { port: 8080, metricsPort: 9091 },
    integrations: { mcpEnabled: false, vectorSearchEnabled: false },
  });
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ success: boolean; message: string } | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const updateConfig = (path: string, value: unknown) => {
    setConfig((prev) => {
      const keys = path.split('.');
      const newConfig = { ...prev };
      let current: Record<string, unknown> = newConfig;

      for (let i = 0; i < keys.length - 1; i++) {
        current[keys[i]] = { ...(current[keys[i]] as Record<string, unknown>) };
        current = current[keys[i]] as Record<string, unknown>;
      }

      current[keys[keys.length - 1]] = value;
      return newConfig;
    });
  };

  const testConnection = async () => {
    setTesting(true);
    setTestResult(null);

    try {
      const res = await fetch(`${API_BASE}/api/setup/test-connection`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config.storage),
      });

      const data = await res.json();
      setTestResult(data);
    } catch {
      setTestResult({ success: false, message: 'Connection failed' });
    } finally {
      setTesting(false);
    }
  };

  const handleSubmit = async () => {
    setSubmitting(true);
    setError(null);

    try {
      const res = await fetch(`${API_BASE}/api/setup`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config),
      });

      const data = await res.json();

      if (data.success) {
        // Show success, wait for restart
        setStep(TOTAL_STEPS + 1); // Success step
        setTimeout(() => {
          onComplete();
        }, 3000);
      } else {
        setError(data.message);
      }
    } catch {
      setError('Setup failed. Please try again.');
    } finally {
      setSubmitting(false);
    }
  };

  const canProceed = (): boolean => {
    switch (step) {
      case 1:
        return true;
      case 2:
        return !!config.deploymentMode;
      case 3:
        if (config.storage?.type === 'postgres') {
          return !!config.storage?.connectionString;
        }
        return true;
      case 4:
        return !!(
          config.admin?.username &&
          config.admin?.password &&
          config.admin.password.length >= 8
        );
      case 5:
        return true;
      case 6:
        return true;
      default:
        return false;
    }
  };

  return (
    <div className="min-h-screen bg-background flex items-center justify-center p-4">
      <Card className="max-w-2xl w-full">
        <CardHeader>
          <div className="flex items-center gap-4 mb-4">
            <div className="w-12 h-12 rounded-lg bg-primary/10 flex items-center justify-center">
              <Rocket className="h-6 w-6 text-primary" />
            </div>
            <div>
              <CardTitle className="text-2xl">Welcome to TopGun</CardTitle>
              <p className="text-muted-foreground">
                Let's set up your server in {TOTAL_STEPS} simple steps
              </p>
            </div>
          </div>
          <Progress value={(step / TOTAL_STEPS) * 100} className="h-2" />
          <p className="text-sm text-muted-foreground mt-2">
            Step {Math.min(step, TOTAL_STEPS)} of {TOTAL_STEPS}
          </p>
        </CardHeader>

        <CardContent className="space-y-6">
          {/* Step 1: Welcome */}
          {step === 1 && (
            <div className="space-y-4">
              <h2 className="text-xl font-semibold">Getting Started</h2>
              <p className="text-muted-foreground">
                TopGun is a hybrid offline-first in-memory data grid with CRDT conflict resolution
                and real-time sync.
              </p>
              <div className="bg-muted/50 p-4 rounded-lg">
                <p className="text-sm">
                  <strong>Tip:</strong> For automated deployments, set{' '}
                  <code className="text-xs bg-muted px-1 py-0.5 rounded">
                    TOPGUN_AUTO_SETUP=true
                  </code>
                </p>
              </div>
            </div>
          )}

          {/* Step 2: Deployment Mode */}
          {step === 2 && (
            <div className="space-y-4">
              <h2 className="text-xl font-semibold">Deployment Mode</h2>
              <RadioGroup
                value={config.deploymentMode}
                onValueChange={(v) => updateConfig('deploymentMode', v)}
                className="space-y-3"
              >
                <label
                  className={cn(
                    'flex items-start gap-4 p-4 border-2 rounded-lg cursor-pointer transition-all',
                    config.deploymentMode === 'standalone'
                      ? 'border-primary bg-primary/5'
                      : 'border-border hover:border-primary/50'
                  )}
                >
                  <RadioGroupItem value="standalone" className="mt-1" />
                  <div>
                    <div className="font-medium">Standalone (Development)</div>
                    <p className="text-sm text-muted-foreground">
                      Single node, SQLite/PostgreSQL, perfect for development and small deployments.
                    </p>
                  </div>
                </label>

                <label
                  className={cn(
                    'flex items-start gap-4 p-4 border-2 rounded-lg cursor-pointer transition-all',
                    config.deploymentMode === 'cluster'
                      ? 'border-primary bg-primary/5'
                      : 'border-border hover:border-primary/50'
                  )}
                >
                  <RadioGroupItem value="cluster" className="mt-1" />
                  <div>
                    <div className="font-medium">Cluster (Production)</div>
                    <p className="text-sm text-muted-foreground">
                      Multi-node with 271 partitions, replication, and high availability.
                    </p>
                  </div>
                </label>
              </RadioGroup>
            </div>
          )}

          {/* Step 3: Storage */}
          {step === 3 && (
            <div className="space-y-4">
              <h2 className="text-xl font-semibold">Storage Configuration</h2>

              <div className="space-y-2">
                <Label>Storage Type</Label>
                <RadioGroup
                  value={config.storage?.type}
                  onValueChange={(v) => updateConfig('storage.type', v)}
                  className="grid grid-cols-3 gap-2"
                >
                  {[
                    { value: 'sqlite', label: 'SQLite', desc: 'File-based, dev only' },
                    { value: 'postgres', label: 'PostgreSQL', desc: 'Recommended' },
                    { value: 'memory', label: 'Memory', desc: 'Testing only' },
                  ].map((opt) => (
                    <label
                      key={opt.value}
                      className={cn(
                        'flex flex-col items-center p-3 border-2 rounded-lg cursor-pointer text-center',
                        config.storage?.type === opt.value
                          ? 'border-primary bg-primary/5'
                          : 'border-border'
                      )}
                    >
                      <RadioGroupItem value={opt.value} className="sr-only" />
                      <span className="font-medium">{opt.label}</span>
                      <span className="text-xs text-muted-foreground">{opt.desc}</span>
                    </label>
                  ))}
                </RadioGroup>
              </div>

              {config.storage?.type === 'postgres' && (
                <div className="space-y-2">
                  <Label>Connection String</Label>
                  <Input
                    type="text"
                    placeholder="postgresql://user:password@localhost:5432/topgun"
                    value={config.storage?.connectionString || ''}
                    onChange={(e) => updateConfig('storage.connectionString', e.target.value)}
                    className="font-mono text-sm"
                  />
                  <div className="flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={testConnection}
                      disabled={testing || !config.storage?.connectionString}
                    >
                      {testing ? (
                        <>
                          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                          Testing...
                        </>
                      ) : (
                        'Test Connection'
                      )}
                    </Button>
                    {testResult && (
                      <span
                        className={cn(
                          'text-sm flex items-center gap-1',
                          testResult.success ? 'text-green-600' : 'text-red-600'
                        )}
                      >
                        {testResult.success ? (
                          <Check className="h-4 w-4" />
                        ) : (
                          <X className="h-4 w-4" />
                        )}
                        {testResult.message}
                      </span>
                    )}
                  </div>
                </div>
              )}

              {config.storage?.type === 'sqlite' && (
                <div className="space-y-2">
                  <Label>Data Directory</Label>
                  <Input
                    type="text"
                    placeholder="./data"
                    value={config.storage?.dataDir || './data'}
                    onChange={(e) => updateConfig('storage.dataDir', e.target.value)}
                    className="font-mono text-sm"
                  />
                </div>
              )}
            </div>
          )}

          {/* Step 4: Admin User */}
          {step === 4 && (
            <div className="space-y-4">
              <h2 className="text-xl font-semibold">Create Admin User</h2>
              <p className="text-sm text-muted-foreground">
                This account will have full access to the admin dashboard.
              </p>

              <div className="space-y-4">
                <div className="space-y-2">
                  <Label>Username</Label>
                  <Input
                    type="text"
                    placeholder="admin"
                    value={config.admin?.username || ''}
                    onChange={(e) => updateConfig('admin.username', e.target.value)}
                  />
                </div>

                <div className="space-y-2">
                  <Label>Password</Label>
                  <Input
                    type="password"
                    placeholder="********"
                    value={config.admin?.password || ''}
                    onChange={(e) => updateConfig('admin.password', e.target.value)}
                  />
                  <p className="text-xs text-muted-foreground">Minimum 8 characters</p>
                </div>

                <div className="space-y-2">
                  <Label>Email (optional)</Label>
                  <Input
                    type="email"
                    placeholder="admin@example.com"
                    value={config.admin?.email || ''}
                    onChange={(e) => updateConfig('admin.email', e.target.value)}
                  />
                </div>
              </div>
            </div>
          )}

          {/* Step 5: Integrations */}
          {step === 5 && (
            <div className="space-y-4">
              <h2 className="text-xl font-semibold">AI & Integrations</h2>

              {/* MCP */}
              <div className="border rounded-lg p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="font-medium">Model Context Protocol (MCP)</div>
                    <p className="text-sm text-muted-foreground">
                      Expose data to Claude Desktop and Cursor
                    </p>
                  </div>
                  <Switch
                    checked={config.integrations?.mcpEnabled}
                    onCheckedChange={(v) => updateConfig('integrations.mcpEnabled', v)}
                  />
                </div>

                {config.integrations?.mcpEnabled && (
                  <div className="pl-4 border-l-2 space-y-2">
                    <div className="space-y-1">
                      <Label className="text-sm">MCP Port</Label>
                      <Input
                        type="number"
                        value={config.integrations?.mcpPort || 3001}
                        onChange={(e) =>
                          updateConfig('integrations.mcpPort', parseInt(e.target.value))
                        }
                        className="w-32"
                      />
                    </div>
                  </div>
                )}
              </div>

              {/* Vector Search */}
              <div className="border rounded-lg p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="font-medium">Vector Search (Phase 15)</div>
                    <p className="text-sm text-muted-foreground">
                      Semantic AI search using embeddings
                    </p>
                  </div>
                  <Switch
                    checked={config.integrations?.vectorSearchEnabled}
                    onCheckedChange={(v) => updateConfig('integrations.vectorSearchEnabled', v)}
                  />
                </div>

                {config.integrations?.vectorSearchEnabled && (
                  <div className="pl-4 border-l-2 space-y-2">
                    <div className="space-y-1">
                      <Label className="text-sm">Embedding Model</Label>
                      <RadioGroup
                        value={config.integrations?.vectorModel || 'local'}
                        onValueChange={(v) => updateConfig('integrations.vectorModel', v)}
                        className="flex gap-4"
                      >
                        <label className="flex items-center gap-2">
                          <RadioGroupItem value="local" />
                          <span className="text-sm">Local (MiniLM-L6)</span>
                        </label>
                        <label className="flex items-center gap-2">
                          <RadioGroupItem value="ollama" />
                          <span className="text-sm">Ollama</span>
                        </label>
                      </RadioGroup>
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Step 6: Review */}
          {step === 6 && (
            <div className="space-y-4">
              <h2 className="text-xl font-semibold">Review Configuration</h2>

              <div className="bg-muted/50 p-4 rounded-lg font-mono text-xs overflow-auto max-h-64">
                <pre>{JSON.stringify(config, null, 2)}</pre>
              </div>

              {error && (
                <div className="bg-destructive/10 text-destructive p-3 rounded-lg text-sm">
                  {error}
                </div>
              )}

              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  const blob = new Blob([JSON.stringify(config, null, 2)], {
                    type: 'application/json',
                  });
                  const url = URL.createObjectURL(blob);
                  const a = document.createElement('a');
                  a.href = url;
                  a.download = 'topgun-config.json';
                  a.click();
                }}
              >
                <Download className="mr-2 h-4 w-4" />
                Download Config (backup)
              </Button>
            </div>
          )}

          {/* Success Step */}
          {step === TOTAL_STEPS + 1 && (
            <div className="text-center space-y-4 py-8">
              <div className="w-16 h-16 mx-auto rounded-full bg-green-100 dark:bg-green-900 flex items-center justify-center">
                <Check className="h-8 w-8 text-green-600 dark:text-green-400" />
              </div>
              <h2 className="text-xl font-semibold">Setup Complete!</h2>
              <p className="text-muted-foreground">
                Server is restarting... You'll be redirected automatically.
              </p>
              <Loader2 className="h-8 w-8 mx-auto animate-spin text-primary" />
            </div>
          )}

          {/* Navigation */}
          {step <= TOTAL_STEPS && (
            <div className="flex justify-between pt-4">
              <Button variant="outline" onClick={() => setStep((s) => s - 1)} disabled={step === 1}>
                Back
              </Button>

              {step < TOTAL_STEPS ? (
                <Button onClick={() => setStep((s) => s + 1)} disabled={!canProceed()}>
                  Continue
                </Button>
              ) : (
                <Button onClick={handleSubmit} disabled={submitting || !canProceed()}>
                  {submitting ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      Setting up...
                    </>
                  ) : (
                    'Finish Setup'
                  )}
                </Button>
              )}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

export default SetupWizard;
