import { useState, useEffect, useCallback } from 'react';
import Editor from '@monaco-editor/react';
import { cn } from '@/lib/utils';

interface JsonEditorProps {
  value: unknown;
  onChange: (value: unknown) => void;
  className?: string;
  readOnly?: boolean;
}

export function JsonEditor({ value, onChange, className, readOnly }: JsonEditorProps) {
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    try {
      setText(JSON.stringify(value, null, 2));
      setError(null);
    } catch {
      setError('Invalid JSON');
    }
  }, [value]);

  const handleChange = useCallback((newText: string | undefined) => {
    if (newText === undefined) return;
    setText(newText);
    try {
      const parsed = JSON.parse(newText);
      onChange(parsed);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Invalid JSON');
    }
  }, [onChange]);

  // Detect dark mode
  const isDark = typeof document !== 'undefined' &&
    document.documentElement.classList.contains('dark');

  return (
    <div className={cn('space-y-2', className)}>
      <div className={cn(
        'rounded-lg overflow-hidden border',
        error && 'border-destructive'
      )}>
        <Editor
          height="300px"
          language="json"
          theme={isDark ? 'vs-dark' : 'light'}
          value={text}
          onChange={handleChange}
          options={{
            readOnly,
            minimap: { enabled: false },
            fontSize: 13,
            fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
            lineNumbers: 'on',
            scrollBeyondLastLine: false,
            automaticLayout: true,
            tabSize: 2,
            wordWrap: 'on',
            formatOnPaste: true,
            formatOnType: true,
          }}
        />
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}

export default JsonEditor;
