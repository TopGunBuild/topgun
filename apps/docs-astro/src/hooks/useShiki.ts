import { useState, useEffect } from 'react';
import { getHighlighter } from '../lib/highlighter';

interface UseShikiResult {
  html: string | null;
  error: Error | null;
  isLoading: boolean;
}

export function useShiki(code: string, language: string = 'bash', theme: string = 'vitesse-dark'): UseShikiResult {
  const [html, setHtml] = useState<string | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let mounted = true;
    setIsLoading(true);
    setError(null);

    async function highlight() {
      try {
        const highlighter = await getHighlighter();
        const highlighted = highlighter.codeToHtml(code, {
          lang: language,
          theme: theme
        });
        if (mounted) {
          setHtml(highlighted);
          setIsLoading(false);
        }
      } catch (e) {
        console.error('Failed to highlight code:', e);
        if (mounted) {
          setError(e instanceof Error ? e : new Error('Failed to highlight code'));
          setIsLoading(false);
        }
      }
    }

    highlight();

    return () => {
      mounted = false;
    };
  }, [code, language, theme]);

  return { html, error, isLoading };
}

