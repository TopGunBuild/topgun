import { useState, useEffect } from 'react';
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

  const handleChange = (newText: string) => {
    setText(newText);
    try {
      const parsed = JSON.parse(newText);
      onChange(parsed);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Invalid JSON');
    }
  };

  return (
    <div className={cn('space-y-2', className)}>
      <textarea
        value={text}
        onChange={(e) => handleChange(e.target.value)}
        readOnly={readOnly}
        className={cn(
          'w-full h-full font-mono text-sm bg-muted p-4 rounded-lg resize-none',
          'focus:outline-none focus:ring-2 focus:ring-ring',
          error && 'ring-2 ring-destructive',
          readOnly && 'cursor-not-allowed opacity-75'
        )}
        spellCheck={false}
      />
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}

export default JsonEditor;
