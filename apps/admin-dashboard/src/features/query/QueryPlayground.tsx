import { useState, useCallback, useRef } from 'react';
import Editor from '@monaco-editor/react';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { DataTable } from '@/components/DataTable';
import { client } from '@/lib/client';
import { Play, Loader2, Clock, AlertCircle } from 'lucide-react';

const EXAMPLE_QUERY = `// Example: Query all entries from a map
// Access the TopGun client via 'client'

// Get all entries from 'users' map
const users = await client.map('users').get();
return users;

// Or use a filter:
// const activeUsers = await client.query('users', {
//   filter: { status: 'active' },
//   limit: 10
// });
// return activeUsers;`;

export function QueryPlayground() {
  const [code, setCode] = useState(EXAMPLE_QUERY);
  const [result, setResult] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [timing, setTiming] = useState<number | null>(null);
  const editorRef = useRef<unknown>(null);

  const executeQuery = useCallback(async () => {
    setLoading(true);
    setError(null);
    setResult(null);

    const start = performance.now();

    try {
      // Create a safe execution context with async wrapper
      const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
      const fn = new AsyncFunction('client', code);
      const queryResult = await fn(client);
      setResult(queryResult);
      setTiming(performance.now() - start);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Unknown error';
      setError(message);
      setTiming(performance.now() - start);
    } finally {
      setLoading(false);
    }
  }, [code]);

  const handleEditorMount = (editor: unknown) => {
    editorRef.current = editor;
  };

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        executeQuery();
      }
    },
    [executeQuery]
  );

  const resultArray = Array.isArray(result) ? result : result ? [result] : [];
  const resultColumns =
    resultArray.length > 0 && typeof resultArray[0] === 'object'
      ? Object.keys(resultArray[0] as Record<string, unknown>).map((key) => ({
          key,
          label: key,
        }))
      : [];

  return (
    <div className="grid grid-rows-2 h-full gap-4 p-4" onKeyDown={handleKeyDown}>
      {/* Query Editor */}
      <Card className="flex flex-col overflow-hidden">
        <CardHeader className="pb-2 flex-shrink-0">
          <div className="flex items-center justify-between">
            <CardTitle className="text-lg">Query</CardTitle>
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground">Cmd+Enter to run</span>
              <Button onClick={executeQuery} disabled={loading} size="sm">
                {loading ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Running...
                  </>
                ) : (
                  <>
                    <Play className="mr-2 h-4 w-4" />
                    Execute
                  </>
                )}
              </Button>
            </div>
          </div>
        </CardHeader>
        <CardContent className="p-0 flex-1 min-h-0">
          <Editor
            height="100%"
            language="javascript"
            theme="vs-dark"
            value={code}
            onChange={(v) => setCode(v || '')}
            onMount={handleEditorMount}
            options={{
              minimap: { enabled: false },
              fontSize: 14,
              fontFamily: 'JetBrains Mono, Fira Code, Monaco, monospace',
              lineNumbers: 'on',
              scrollBeyondLastLine: false,
              automaticLayout: true,
              tabSize: 2,
              wordWrap: 'on',
              padding: { top: 16 },
            }}
          />
        </CardContent>
      </Card>

      {/* Results */}
      <Card className="flex flex-col overflow-hidden">
        <CardHeader className="pb-2 flex-shrink-0">
          <div className="flex items-center justify-between">
            <CardTitle className="text-lg">Results</CardTitle>
            {timing !== null && (
              <span className="text-sm text-muted-foreground flex items-center gap-1">
                <Clock className="h-4 w-4" />
                {timing.toFixed(2)}ms
              </span>
            )}
          </div>
        </CardHeader>
        <CardContent className="flex-1 min-h-0 overflow-auto">
          {error ? (
            <div className="p-4 text-destructive bg-destructive/10 rounded-lg font-mono text-sm flex items-start gap-2">
              <AlertCircle className="h-5 w-5 mt-0.5 flex-shrink-0" />
              <pre className="whitespace-pre-wrap">{error}</pre>
            </div>
          ) : result !== null ? (
            <Tabs defaultValue="table" className="h-full flex flex-col">
              <TabsList className="flex-shrink-0">
                <TabsTrigger value="table">Table</TabsTrigger>
                <TabsTrigger value="json">JSON</TabsTrigger>
                <TabsTrigger value="stats">Stats</TabsTrigger>
              </TabsList>

              <TabsContent value="table" className="flex-1 overflow-auto mt-4">
                {resultArray.length > 0 && resultColumns.length > 0 ? (
                  <DataTable data={resultArray} columns={resultColumns} />
                ) : (
                  <pre className="text-sm font-mono bg-muted p-4 rounded-lg">
                    {JSON.stringify(result, null, 2)}
                  </pre>
                )}
              </TabsContent>

              <TabsContent value="json" className="flex-1 overflow-auto mt-4">
                <pre className="text-sm font-mono bg-muted p-4 rounded-lg overflow-auto">
                  {JSON.stringify(result, null, 2)}
                </pre>
              </TabsContent>

              <TabsContent value="stats" className="mt-4">
                <div className="space-y-2 text-sm">
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Type:</span>
                    <span>{Array.isArray(result) ? 'Array' : typeof result}</span>
                  </div>
                  {Array.isArray(result) && (
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Count:</span>
                      <span>{result.length} records</span>
                    </div>
                  )}
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Duration:</span>
                    <span>{timing?.toFixed(2)}ms</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Size:</span>
                    <span>{JSON.stringify(result).length} bytes</span>
                  </div>
                </div>
              </TabsContent>
            </Tabs>
          ) : (
            <div className="flex items-center justify-center h-full text-muted-foreground">
              Run a query to see results
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

export default QueryPlayground;
